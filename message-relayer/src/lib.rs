//! USC write-ability message relayer.
//!
//! This crate is the Phase-1 PoC — it observes attestor votes on a libp2p mesh, aggregates them up
//! to the configured threshold, and submits `Inbox.deliverMessage(...)` on the destination
//! chains for one or more `(creditcoin_chain_key → destination_chain)` routes.
//!
//! The runtime is a small set of `tokio::spawn` tasks coordinated by `mpsc` channels:
//!
//!  * `events::watch_outbox` (per route) — reads `MessagePublished` from Creditcoin L1 EVM,
//!    pushes [`IndexedMessage`]s into the pool's "chain-first allowlist".
//!  * `p2p::run` (one shared swarm) — subscribes to `{chain_key}/message-votes/v1` topics,
//!    decodes [`MessageVote`] envelopes, forwards to the pool.
//!  * `pool::run` — applies PoC §6.2 validation rules and dispatches a [`DeliveryJob`] per
//!    route once `2N/3 + 1` distinct signers have been observed.
//!  * `delivery::run` (per route) — simulates, sends, classifies the outcome, retries.
//!  * Plus `prom::build_router` + a hardware-stats updater.
//!
//! Shutdown is one [`CancellationToken`] fanned out to every task; on Ctrl+C / SIGTERM we
//! cancel and drain the [`tokio::task::JoinSet`].

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::signal;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub mod abi;
pub mod ack;
pub mod attestor_set;
pub mod checkpoint;
pub mod config;
pub mod delivery;
pub mod events;
pub mod hash;
pub mod p2p;
pub mod pool;
pub mod prom;

pub use config::{
    AttestorSet, AttestorSource, ChainRoute, Config, DeliveryConfig, P2pConfig, VoteCacheConfig,
};
pub use delivery::DeliveryJob;
pub use events::{ConfigOverrideResolver, FactoryResolver, IndexedMessage, OutboxResolver};
pub use p2p::MessageVote;
pub use pool::{calculate_threshold, RouteAttestors};
pub use prom::{Metrics, MetricsTrait, NoopMetrics, RelayerMetrics};

/// Capacities tuned for "indexer pace": Outbox events are slow, votes can spike. Both are
/// well below pool memory bounds (`vote_cache.max_messages * threshold`).
const INDEXED_CHANNEL_CAP: usize = 1_024;
const VOTE_CHANNEL_CAP: usize = 8_192;
const DELIVERY_CHANNEL_CAP: usize = 64;
/// Attestor-set updates are rare (one per on-chain set change); a small buffer is ample.
const SET_UPDATE_CHANNEL_CAP: usize = 16;
/// Reobservation requests are emitted at most once per stalled message per minute; a modest buffer
/// absorbs a burst when many messages stall at once.
const REOBS_CHANNEL_CAP: usize = 256;
/// Read-only vote-bundle queries from the HTTP layer.
const QUERY_CHANNEL_CAP: usize = 64;

/// Top-level relayer process.
pub struct Server {
    config: Config,
    prom_metrics: Arc<RelayerMetrics>,
}

impl Server {
    pub async fn new(config: Config) -> Result<Self> {
        let chain_keys: Vec<u64> = config.routes.iter().map(|r| r.chain_key).collect();
        let prom_metrics = Arc::new(RelayerMetrics::new(&chain_keys));

        info!(
            routes = config.routes.len(),
            ?chain_keys,
            cc3_rpc_url = %redact_url_query(&config.cc3_rpc_url),
            creditcoin_eth_rpc_url = %redact_url_query(&config.creditcoin_eth_rpc_url),
            "🚚 Configured message relayer"
        );
        for route in &config.routes {
            info!(
                chain_key = route.chain_key,
                creditcoin_chain_id = route.creditcoin_chain_id,
                inbox = %route.inbox_address,
                outbox = ?route.outbox_address,
                destination_rpc = %redact_url_query(&route.destination_rpc_url),
                attestor_set = ?attestor_set_summary(&route.attestor_set),
                threshold_override = ?route.threshold_override,
                "📨 Route configured"
            );
        }

        Ok(Self {
            config,
            prom_metrics,
        })
    }

    pub async fn run(self) -> Result<()> {
        let cancel = CancellationToken::new();
        let metrics: Metrics = self.prom_metrics.clone() as Metrics;

        // Persistent block cursors (resume after restart instead of from the chain head). Disabled
        // when `checkpoint_path` is None.
        let checkpoint: Option<Arc<checkpoint::CheckpointStore>> =
            match &self.config.checkpoint_path {
                Some(path) => {
                    let store = checkpoint::CheckpointStore::load(path).with_context(|| {
                        format!("failed to load checkpoint store at {}", path.display())
                    })?;
                    info!(path = %path.display(), "🗂️  block-cursor persistence enabled");
                    Some(Arc::new(store))
                }
                None => {
                    warn!(
                        "checkpoint persistence disabled — watchers start from the chain head and \
                       will skip events emitted while the relayer is down"
                    );
                    None
                }
            };

        // Resolve attestor sets up-front so the pool only sees concrete addresses.
        let mut route_attestors: Vec<RouteAttestors> = Vec::with_capacity(self.config.routes.len());
        for route in &self.config.routes {
            route_attestors.push(resolve_attestors(route)?);
        }

        // Channels.
        let (indexed_tx, indexed_rx) = mpsc::channel::<IndexedMessage>(INDEXED_CHANNEL_CAP);
        let (vote_tx, vote_rx) = mpsc::channel::<MessageVote>(VOTE_CHANNEL_CAP);
        let (set_tx, set_rx) = mpsc::channel::<RouteAttestors>(SET_UPDATE_CHANNEL_CAP);
        let (reobs_tx, reobs_rx) =
            mpsc::channel::<write_ability::envelope::ReobservationRequest>(REOBS_CHANNEL_CAP);
        let (query_tx, query_rx) = mpsc::channel::<pool::PoolQuery>(QUERY_CHANNEL_CAP);
        let mut delivery_txs: HashMap<u64, mpsc::Sender<DeliveryJob>> = HashMap::new();

        let mut tasks = JoinSet::new();

        // Per-route delivery workers.
        for route in &self.config.routes {
            let (dtx, drx) = mpsc::channel::<DeliveryJob>(DELIVERY_CHANNEL_CAP);
            delivery_txs.insert(route.chain_key, dtx);
            spawn_worker(
                &mut tasks,
                format!("delivery worker (chain_key {})", route.chain_key),
                delivery::run(
                    route.clone(),
                    self.config.delivery.clone(),
                    drx,
                    metrics.clone(),
                    cancel.clone(),
                ),
            );
        }

        // Pool.
        spawn_worker(
            &mut tasks,
            "vote pool",
            pool::run(
                route_attestors,
                self.config.vote_cache.clone(),
                pool::PoolHandles {
                    indexed_rx,
                    vote_rx,
                    delivery_txs,
                    set_update_rx: set_rx,
                    reobs_tx,
                    query_rx,
                },
                metrics.clone(),
                cancel.clone(),
            ),
        );

        // Outbox watchers (per route).
        let resolver: Arc<dyn OutboxResolver> = Arc::new(ConfigOverrideResolver);
        for route in &self.config.routes {
            spawn_worker(
                &mut tasks,
                format!("outbox watcher (chain_key {})", route.chain_key),
                events::watch_outbox(
                    route.clone(),
                    self.config.creditcoin_eth_rpc_url.clone(),
                    indexed_tx.clone(),
                    metrics.clone(),
                    resolver.clone(),
                    checkpoint.clone(),
                    cancel.clone(),
                ),
            );
        }
        // Drop the parent indexed_tx so the pool can detect the channel closing once every
        // watcher has exited (avoids a stray clone keeping the pool alive forever).
        drop(indexed_tx);

        // Acknowledgment submitters (per route, opt-in). Each watches the destination Inbox for
        // `MessageDelivered`, fetches a native USC delivery proof, and submits it to the
        // source-chain `AcknowledgmentValidator`. Routes without `ack` config are skipped.
        for route in self.config.routes.iter().filter(|r| r.ack.is_some()) {
            spawn_worker(
                &mut tasks,
                format!("ack submitter (chain_key {})", route.chain_key),
                ack::run(
                    route.clone(),
                    self.config.creditcoin_eth_rpc_url.clone(),
                    checkpoint.clone(),
                    cancel.clone(),
                ),
            );
        }

        // On-chain attestor-set watchers (per route, only for `OnChain` sets). Each polls the
        // destination validator's attestors()/threshold() and hot-reloads the pool on change. Static
        // routes get no watcher (the set never changes), so we don't spawn a task that would exit
        // immediately and trip the supervisor's "a worker exited" teardown.
        for route in self
            .config
            .routes
            .iter()
            .filter(|r| matches!(r.attestor_set, AttestorSet::OnChain { .. }))
        {
            spawn_worker(
                &mut tasks,
                format!("attestor-set watcher (chain_key {})", route.chain_key),
                attestor_set::run(route.clone(), set_tx.clone(), cancel.clone()),
            );
        }
        // Drop the parent set_tx so the pool's set-update branch closes once every watcher exits
        // (and stays inert when there are no on-chain routes at all).
        drop(set_tx);

        // libp2p worker (one shared swarm).
        spawn_worker(
            &mut tasks,
            "libp2p worker",
            p2p::run(
                self.config.p2p.clone(),
                self.config.routes.iter().map(|r| r.chain_key).collect(),
                vote_tx,
                reobs_rx,
                metrics.clone(),
                cancel.clone(),
            ),
        );

        // Hardware metrics gauges (managed worker — drains with the rest on shutdown).
        spawn_worker(
            &mut tasks,
            "hardware metrics updater",
            RelayerMetrics::run_hardware_updater(self.prom_metrics.clone(), cancel.clone()),
        );

        // /metrics + /health.
        let bind_host = &self.config.bind_host;
        let ip =
            bind_host.parse::<IpAddr>().with_context(|| {
                format!(
                    "Invalid bind host: '{bind_host}'. Expected IP address (e.g. '0.0.0.0', '::1')",
                )
            })?;
        let bind_addr = SocketAddr::new(ip, self.config.bind_port);
        let app = prom::build_router(self.prom_metrics.clone(), query_tx);
        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind HTTP listener at {bind_addr}"))?;
        info!("🌐 Metrics + health endpoint listening on {bind_addr}");
        let axum_cancel = cancel.clone();
        spawn_worker(&mut tasks, "HTTP server", async move {
            let shutdown = async move { axum_cancel.cancelled().await };
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await?;
            Ok(())
        });

        info!("✅ All workers online");

        // Wait for either an external shutdown signal or for any worker to exit (which we treat
        // as a fatal indication that something is wrong — e.g. a watcher's RPC died).
        tokio::select! {
            () = shutdown_signal() => {
                info!("🛑 Global shutdown requested");
            }
            _ = tasks.join_next() => {
                warn!("a worker exited; tearing down the rest");
            }
        }
        cancel.cancel();
        while tasks.join_next().await.is_some() {}
        info!("🛑 message relayer drained, exiting");
        Ok(())
    }

    /// Borrow the validated configuration. Used by tests to inspect the loaded values.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// Spawn a fallible worker future onto `tasks`, tagging it with `label` for diagnostics.
///
/// Every worker treats a clean `Ok(())` as an orderly shutdown (it observed the shared cancel
/// token); an `Err` means it died unexpectedly — a dropped RPC, a closed channel — and is logged
/// here so the failure is attributable to a specific worker. The spawned task is `()`-typed so the
/// outer [`JoinSet`] drives every worker uniformly; the `select!` on `join_next` in
/// [`Server::run`] then tears the rest down regardless of which one exited.
fn spawn_worker(
    tasks: &mut JoinSet<()>,
    label: impl Into<String>,
    fut: impl Future<Output = Result<()>> + Send + 'static,
) {
    let label = label.into();
    tasks.spawn(async move {
        if let Err(err) = fut.await {
            error!(worker = %label, %err, "worker exited with error");
        }
    });
}

/// Convert an [`AttestorSet`] into the concrete [`RouteAttestors`] the pool expects. Static sets
/// translate directly; on-chain sets are not implemented in PoC and yield an explanatory error.
fn resolve_attestors(route: &ChainRoute) -> Result<RouteAttestors> {
    match &route.attestor_set {
        AttestorSet::Static(addrs) => {
            let attestors = addrs.clone();
            let n = attestors.len();
            let threshold = route
                .threshold_override
                .map(|t| t as usize)
                .unwrap_or_else(|| calculate_threshold(n));
            // The pool enforces THIS threshold locally; the destination `EOAValidator` enforces its
            // own `threshold()`. For a static route the two are configured independently, so a
            // mismatch is silent until delivery — too high assembles bundles the Inbox rejects, too
            // low wastes delivery attempts. (On-chain routes avoid this by reading `threshold()`
            // live; see `attestor_set::run`.) Surface the value so an operator can confirm parity
            // with the deployed validator.
            warn!(
                chain_key = route.chain_key,
                attestors = n,
                threshold,
                threshold_override = ?route.threshold_override,
                "static attestor-set route — this threshold MUST equal the on-chain EOAValidator.threshold(); verify they match or delivery will revert"
            );
            Ok(RouteAttestors {
                chain_key: route.chain_key,
                attestors,
                threshold,
            })
        }
        // On-chain sets are resolved live by the per-route attestor-set watcher (see
        // [`attestor_set::run`]). Start empty with an unreachable threshold so the pool accepts
        // nothing until the first on-chain read arrives — votes meanwhile are simply re-gossiped.
        AttestorSet::OnChain { .. } => Ok(RouteAttestors {
            chain_key: route.chain_key,
            attestors: Vec::new(),
            threshold: usize::MAX,
        }),
    }
}

/// Mask `?query` parameters so URL keys do not appear in logs.
fn redact_url_query(url: &str) -> String {
    url.split_once('?')
        .map(|(base, _)| format!("{base}?…"))
        .unwrap_or_else(|| url.to_string())
}

fn attestor_set_summary(set: &AttestorSet) -> String {
    match set {
        AttestorSet::Static(addresses) => format!("static({} addresses)", addresses.len()),
        AttestorSet::OnChain { source } => match source {
            AttestorSource::Evm { address } => format!("evm_contract({address})"),
            AttestorSource::Cc3 { chain_key } => format!("cc3_active_set(chain_key={chain_key})"),
        },
    }
}

/// Resolve a Ctrl+C or SIGTERM and return.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }

    info!("Shutdown signal received");
}
