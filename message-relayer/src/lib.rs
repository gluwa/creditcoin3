//! USC write-ability message relayer.
//!
//! See `relayer-poc.pdf` (repo root) and `PLAN.md` (this crate) for the full design. This
//! crate is the Phase-1 PoC — it observes attestor votes on a libp2p mesh, aggregates them up
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
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use alloy::primitives::Address;
use anyhow::{bail, Context, Result};
use tokio::signal;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub mod abi;
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

        // Resolve attestor sets up-front so the pool only sees concrete addresses.
        let mut route_attestors: Vec<RouteAttestors> = Vec::with_capacity(self.config.routes.len());
        for route in &self.config.routes {
            route_attestors.push(resolve_attestors(route)?);
        }

        // Channels.
        let (indexed_tx, indexed_rx) = mpsc::channel::<IndexedMessage>(INDEXED_CHANNEL_CAP);
        let (vote_tx, vote_rx) = mpsc::channel::<MessageVote>(VOTE_CHANNEL_CAP);
        let mut delivery_txs: HashMap<u64, mpsc::Sender<DeliveryJob>> = HashMap::new();

        let mut tasks = JoinSet::new();

        // Per-route delivery workers.
        for route in &self.config.routes {
            let (dtx, drx) = mpsc::channel::<DeliveryJob>(DELIVERY_CHANNEL_CAP);
            delivery_txs.insert(route.chain_key, dtx);
            let r = route.clone();
            let dc = self.config.delivery.clone();
            let m = metrics.clone();
            let c = cancel.clone();
            tasks.spawn(async move {
                if let Err(err) = delivery::run(r.clone(), dc, drx, m, c).await {
                    error!(chain_key = r.chain_key, %err, "delivery worker exited with error");
                }
            });
        }

        // Pool.
        {
            let m = metrics.clone();
            let c = cancel.clone();
            let cache = self.config.vote_cache.clone();
            let handles = pool::PoolHandles {
                indexed_rx,
                vote_rx,
                delivery_txs,
            };
            tasks.spawn(async move {
                if let Err(err) = pool::run(route_attestors, cache, handles, m, c).await {
                    error!(%err, "vote pool exited with error");
                }
            });
        }

        // Outbox watchers (per route).
        let resolver: Arc<dyn OutboxResolver> = Arc::new(ConfigOverrideResolver);
        for route in &self.config.routes {
            let r = route.clone();
            let url = self.config.creditcoin_eth_rpc_url.clone();
            let tx = indexed_tx.clone();
            let m = metrics.clone();
            let c = cancel.clone();
            let res = resolver.clone();
            tasks.spawn(async move {
                if let Err(err) = events::watch_outbox(r.clone(), url, tx, m, res, c).await {
                    error!(chain_key = r.chain_key, %err, "outbox watcher exited with error");
                }
            });
        }
        // Drop the parent indexed_tx so the pool can detect the channel closing once every
        // watcher has exited (avoids a stray clone keeping the pool alive forever).
        drop(indexed_tx);

        // libp2p worker (one shared swarm).
        {
            let cfg = self.config.p2p.clone();
            let chain_keys: Vec<u64> = self.config.routes.iter().map(|r| r.chain_key).collect();
            let m = metrics.clone();
            let c = cancel.clone();
            tasks.spawn(async move {
                if let Err(err) = p2p::run(cfg, chain_keys, vote_tx, m, c).await {
                    error!(%err, "libp2p worker exited with error");
                }
            });
        }

        // Hardware metrics gauges.
        RelayerMetrics::spawn_hardware_updater(self.prom_metrics.clone());

        // /metrics + /health.
        let bind_host = &self.config.bind_host;
        let ip =
            bind_host.parse::<IpAddr>().with_context(|| {
                format!(
                    "Invalid bind host: '{bind_host}'. Expected IP address (e.g. '0.0.0.0', '::1')",
                )
            })?;
        let bind_addr = SocketAddr::new(ip, self.config.bind_port);
        let app = prom::build_router(self.prom_metrics.clone());
        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind HTTP listener at {bind_addr}"))?;
        info!("🌐 Metrics + health endpoint listening on {bind_addr}");
        let axum_cancel = cancel.clone();
        tasks.spawn(async move {
            let shutdown = async move { axum_cancel.cancelled().await };
            if let Err(err) = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
            {
                error!(%err, "HTTP server exited with error");
            }
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

/// Convert an [`AttestorSet`] into the concrete [`RouteAttestors`] the pool expects. Static sets
/// translate directly; on-chain sets are not implemented in PoC and yield an explanatory error.
fn resolve_attestors(route: &ChainRoute) -> Result<RouteAttestors> {
    let attestors: Vec<Address> = match &route.attestor_set {
        AttestorSet::Static(addrs) => addrs.clone(),
        AttestorSet::OnChain { source } => {
            bail!(
                "chain_key {}: on-chain attestor resolution ({:?}) is not implemented in PoC \
                 — configure `attestor_set: kind: static` for now",
                route.chain_key,
                source
            );
        }
    };
    let n = attestors.len();
    let threshold = route
        .threshold_override
        .map(|t| t as usize)
        .unwrap_or_else(|| calculate_threshold(n));
    Ok(RouteAttestors {
        chain_key: route.chain_key,
        attestors,
        threshold,
    })
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
