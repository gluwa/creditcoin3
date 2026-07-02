//! attestor — async tasks, JoinSet supervisor, lightweight-vote protocol.
//!
//! # Layout
//!
//! - [`Attestor::run`] is the orchestrator. It does the synchronous startup work
//!   (resolve endpoints, register BLS, wait for eligibility, fetch chain config), constructs
//!   the [`shared::Shared`] state, then spawns four async tasks on a single tokio runtime:
//!   `api`, `p2p`, `production`, `validation`.
//! - There is **one** `Arc<cc_client::Client>` — every task shares it, so any `cc3.reconnect()`
//!   from any task is observed everywhere (the inner `ArcSwap` is shared).
//! - There are **no** OS threads spawned for workers. No per-worker tokio runtime.
//! - There is **one** CC3 finality subscription, owned by the production task. The validation
//!   task watches `latest_finalized` via a `watch::Receiver` instead of subscribing again.
//! - Cancellation is a single [`tokio_util::sync::CancellationToken`]. Ctrl+C is handled here
//!   in `run` — tasks just observe `token.cancelled()`. No `Interrupt::Stop` propagation.

pub mod attestation;
pub mod bls;
pub mod error;
pub mod health;
pub mod proof_cache;
pub mod retry;
pub mod secret;
pub mod shared;
pub mod startup;
pub mod tasks;
pub mod vote;

pub use error::Error;

use std::num::NonZero;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::shared::{AttestationInfo, Shared};

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(builder::Builder)]
pub struct Config {
    name: String,
    chain_key: attestor_primitives::ChainKey,
    stream: secret::Config,
    attestation: attestation::Config,
    p2p: tasks::p2p::ConfigIncomplete,
    api: tasks::api::ConfigIncomplete,
}

// ---------------------------------------- [ Attestor ] ---------------------------------------- //

pub struct Attestor {
    config: Config,
}

impl Attestor {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    #[tracing::instrument(
        name = "attestor",
        skip_all,
        fields(name = self.config.name, chain_key = %self.config.chain_key),
    )]
    pub async fn run(self) -> Result<(), Error> {
        let token = CancellationToken::new();

        // Watch for Ctrl+C / SIGTERM from the very first await so a shutdown during the
        // synchronous startup phase (waiting on RPC endpoints, election) takes effect promptly
        // instead of hanging until the task supervisor is up. The supervisor below also selects
        // on `ctrl_c`; both cancelling the same token is idempotent.
        tokio::spawn({
            let token = token.clone();
            async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    token.cancel();
                }
            }
        });

        // ----------------------------------* identity *--------------------------------------- //

        let chain_key = self.config.chain_key;

        let secret_str = self.config.stream.secret.to_secret_uri_string();
        let signer = cc_client::signer::CC3Signer::new(secret_str.as_str()).map_err(Error::Init)?;
        let account_id = signer.account_id();
        let attestor_id = attestor_primitives::AttestorId::from_public(account_id.0);

        let mut seed = self.config.stream.secret.to_seed_bytes_32();
        let keypair_p2p =
            libp2p::identity::Keypair::ed25519_from_bytes(&mut *seed).expect("ed25519 keypair");
        let peer_id = libp2p::PeerId::from_public_key(&keypair_p2p.public());

        let bls_seed = self.config.stream.secret.to_bls_seed_bytes();
        let bls_key = bls_signatures::PrivateKey::new(bls_seed.as_slice());

        tracing::info!(name = %self.config.name, %account_id, %chain_key, "🙋‍♀️ starting attestor");

        // ----------------------------------* endpoints *-------------------------------------- //

        match startup::wait_for_endpoints(
            &token,
            &self.config.stream.url_eth,
            &self.config.stream.url_cc3,
        )
        .await
        {
            Ok(()) => {}
            Err(Error::ShutdownDuringStartup) => {
                tracing::info!("🔌 shutdown during startup (endpoint wait)");
                return Ok(());
            }
            Err(e) => return Err(e),
        }

        // ONE cc3 client, shared everywhere.
        let cc3_raw = cc_client::Client::new(
            self.config.stream.url_cc3.as_ref().as_ref(),
            secret_str.as_str(),
        )
        .await
        .map_err(Error::Init)?;
        let cc3 = Arc::new(cc3_raw);

        // Reconcile the metadata the binary was compiled against with the chain's live
        // metadata before doing anything that depends on extrinsic encoding. Applies the
        // chain's metadata into the OnlineClient on a compatible drift; refuses to boot on
        // a breaking Attestation pallet change.
        startup::reconcile_metadata(&cc3).await?;

        let eth = eth::Client::new(self.config.stream.url_eth.as_ref().as_ref(), None)
            .await
            .map_err(Error::Init)?;

        // ----------------------------------* chain config *----------------------------------- //

        let supported_chain = cc3
            .get_supported_chain(chain_key)
            .await?
            .ok_or(Error::ChainKeyNotSupported(chain_key))?;
        if supported_chain.chain_id != eth.chain_id() {
            return Err(Error::ChainIdMismatch {
                runtime: supported_chain.chain_id,
                rpc: eth.chain_id(),
            });
        }

        let strategy: supported_chains_primitives::MaturityStrategy = supported_chain
            .maturity_strategy
            .as_str()
            .try_into()
            .map_err(|e| Error::InvalidMaturityStrategy(chain_key, e))?;
        let maturity_delay = strategy
            .maturity_delay()
            .ok_or(Error::NoMaturityDelayForStrategy(strategy))?;

        // ----------------------------------* balance check *---------------------------------- //

        let free = cc3.get_free_balance(&account_id).await?;
        if free < common::constants::MIN_BALANCE {
            return Err(Error::Init(anyhow::anyhow!(
                "insufficient balance: {free} < {}",
                common::constants::MIN_BALANCE
            )));
        }
        tracing::info!(%account_id, balance = %free, "🔍 balance ok");

        // ----------------------------------* register + eligibility *------------------------- //

        startup::register_bls(chain_key, &cc3, &account_id, &bls_key).await?;
        let attestors = match startup::wait_for_eligible(&token, chain_key, &cc3, &account_id).await
        {
            Ok(a) => a,
            Err(Error::ShutdownDuringStartup) => {
                tracing::info!("🔌 shutdown during startup (election wait)");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let bls_store = Arc::new(
            bls::BlsStore::new(&cc3, &token, chain_key, &attestors)
                .await
                .map_err(Error::Rpc)?,
        );

        // ----------------------------------* attestation params *----------------------------- //

        let interval_attestation =
            if let Some(forced) = self.config.attestation.attestation_interval {
                forced
            } else {
                cc3.chain_attestation_interval(chain_key)
                    .await?
                    .and_then(NonZero::new)
                    .ok_or(Error::MissingAttestationInterval(chain_key))?
            };

        let (genesis_height, start_attestation) =
            startup::fetch_start_point(chain_key, &cc3).await?;
        let start_height = self
            .config
            .attestation
            .start_height
            .or_else(|| start_attestation.as_ref().map(|i| i.height + 1))
            .unwrap_or(genesis_height);

        let target = cc3
            .target_sample_size(chain_key)
            .await
            .map_err(|_| Error::MissingTargetSampleSize(chain_key))?;
        let quorum = NonZero::new(attestor_primitives::calculate_threshold(target) as usize)
            .expect("quorum > 0");

        tracing::info!(
            quorum = %quorum,
            interval = %interval_attestation,
            start_height,
            ?start_attestation,
            "🧑‍🤝‍🧑 chain data");

        // ----------------------------------* shared state *----------------------------------- //

        let proof_cache = proof_cache::ProofCache::new();

        // Metrics
        let metrics_cfg = metrics::ConfigBuilder::new()
            .with_name(self.config.name.clone())
            .with_address(account_id.clone())
            .with_peer_id(peer_id)
            .with_chain_key(chain_key)
            .with_start_height(start_height)
            .with_start_attestation(start_attestation.map(|i| stream::util::AttestationInfo {
                height: i.height,
                digest: i.digest,
            }))
            .with_genesis(genesis_height)
            .with_attestation_latest_eth(start_height)
            .with_attestation_interval(interval_attestation)
            .build();
        let metrics = metrics::Metrics::new(metrics_cfg);

        // Pool — wires its MetricsHook to our Prometheus registry.
        struct PoolMetrics(metrics::Metrics);
        impl attestor_pool::MetricsHook for PoolMetrics {
            fn quorum_delay(&self, elapsed: std::time::Duration) {
                self.0.observe_quorum_delay(elapsed);
            }
            fn invalid_vote(&self) {
                self.0.increase_invalid_attestation_count();
            }
            fn equivocation(&self) {
                self.0.increase_equivocation_count();
            }
        }

        let (pool_send, pool_recv) = attestor_pool::attestation_pool(
            attestor_pool::ConfigBuilder::new()
                .with_attestors(attestors)
                .with_quorum(quorum)
                .with_attestation_interval(interval_attestation)
                .with_start_height(start_height)
                .with_max_catchup(common::constants::MAX_CATCHUP)
                .with_start_digest(start_attestation.map(|i| i.digest))
                .with_start_height_finalized(start_attestation.map(|i| i.height))
                .with_metrics(
                    Box::new(PoolMetrics(metrics.clone())) as Box<dyn attestor_pool::MetricsHook>
                )
                .build(),
        );

        // Channels and watches
        let (gossip_tx, gossip_rx) =
            mpsc::channel::<vote::Vote>(common::constants::CAPACITY_CHANNEL);
        // Production → p2p nudge to evict a chilled/kicked attestor's peer. Unbounded: rare
        // (committee-change) events that must not apply backpressure to the production task.
        let (peer_deactivated_tx, peer_deactivated_rx) =
            mpsc::unbounded_channel::<attestor_primitives::AttestorId>();
        let (can_attest_tx, can_attest_rx) = watch::channel::<bool>(true);
        // `None` until the production task observes the first BlockAttested event. Validation's
        // `wait_for_finalized` helper waits for `Some(info)` with `info.height >= target`, so it
        // can't be tricked by a placeholder zero-digest value.
        let (latest_finalized_tx, latest_finalized_rx) =
            watch::channel::<Option<AttestationInfo>>(start_attestation);

        // `None` until production caches its first local AttestationData. p2p uses this to
        // drain a buffer of votes that arrived ahead of our local production at that height.
        let (local_produced_tx, local_produced_rx) =
            watch::channel::<Option<attestor_primitives::Height>>(None);

        let shared = Arc::new(Shared {
            name: self.config.name.clone(),
            chain_key,
            account_id,
            attestor_id,

            signer,
            bls_key,

            cc3: cc3.clone(),
            eth,

            bls_store,
            metrics,
            health: Arc::new(crate::health::Health::new()),

            pool_send,
            gossip_tx,
            peer_deactivated_tx,

            can_attest_tx,
            can_attest_rx,

            latest_finalized_tx,
            latest_finalized_rx,

            local_produced_tx,
            local_produced_rx,

            proof_cache,

            interval_attestation: parking_lot::RwLock::new(interval_attestation),
            maturity_delay,
            start_height,
            genesis: genesis_height,

            token: token.clone(),
        });

        // ----------------------------------* spawn tasks *------------------------------------ //

        let mut set: JoinSet<Result<&'static str, Error>> = JoinSet::new();

        {
            let shared = shared.clone();
            let cfg = self.config.api.with_metrics(shared.metrics.clone()).build();
            set.spawn(async move { tasks::api::run(shared, cfg).await.map(|_| "api") });
        }

        {
            let shared = shared.clone();
            let cfg = self
                .config
                .p2p
                .with_keypair(keypair_p2p)
                .with_chain_key(chain_key)
                .build();
            set.spawn(async move {
                tasks::p2p::run(shared, cfg, gossip_rx, peer_deactivated_rx)
                    .await
                    .map(|_| "p2p")
            });
        }

        {
            let shared = shared.clone();
            set.spawn(async move {
                tasks::production::run(shared, start_attestation)
                    .await
                    .map(|_| "production")
            });
        }

        {
            let shared = shared.clone();
            set.spawn(async move {
                tasks::validation::run(shared, pool_recv)
                    .await
                    .map(|_| "validation")
            });
        }

        {
            let shared = shared.clone();
            set.spawn(async move {
                tasks::runtime_updater::run(shared)
                    .await
                    .map(|_| "runtime_updater")
            });
        }

        tracing::info!("✅ all services online");

        // ----------------------------------* supervise *-------------------------------------- //
        //
        // The supervision loop is now ~15 lines: select on (ctrl_c, joinset). First
        // ctrl_c → cancel token, drain remaining tasks. First failing task → cancel token,
        // drain. This replaces the v1 `CancellationMonitor + Notify + wait_for_worker`
        // (~100 lines) with this:

        let mut result: Result<(), Error> = Ok(());

        // First signal wins: either ctrl_c, or the first task to exit. Every branch ends the
        // select (no looping) — we then cancel and drain. Tasks are meant to run until
        // `token.cancelled()`, so *any* task exit here is the trigger to shut the whole set down.
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("🔌 shutdown signal");
                token.cancel();
            }
            Some(joined) = set.join_next() => {
                match joined {
                    // A task returning Ok while the token is still live is not a clean shutdown —
                    // it's a core service quietly dying while the rest of the pod (notably the
                    // /metrics endpoint) keeps reporting healthy. Promote it to a failure so we
                    // cancel + drain + exit nonzero and k8s restarts us, rather than limping on
                    // half-dead. If the token is already cancelled, ctrl_c won the race and this
                    // is a legitimate shutdown exit. (No task self-cancels: fatal conditions like
                    // runtime-metadata drift return Err and are handled below.)
                    Ok(Ok(name)) => {
                        if token.is_cancelled() {
                            tracing::info!(task = name, "🟢 task exited after shutdown was requested");
                        } else {
                            tracing::error!(task = name, "⛔ task exited before shutdown was requested");
                            result = Err(Error::TaskExitedEarly(name));
                        }
                    }
                    Ok(Err(err)) => {
                        tracing::error!(%err, "⛔ task failed");
                        result = Err(err);
                    }
                    Err(join_err) => {
                        tracing::error!(%join_err, "⛔ task join error");
                        result = Err(Error::TaskJoin(join_err));
                    }
                }
                // Flip /health to unhealthy now so a livenessProbe restarts us promptly even if
                // the drain below stalls on a wedged sibling task.
                if result.is_err() {
                    shared.health.note_fault();
                }
                token.cancel();
            }
        }

        // Drain remaining tasks.
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(name)) => tracing::info!(task = name, "🟢 task exited cleanly"),
                Ok(Err(err)) => {
                    tracing::error!(%err, "⛔ task failed during shutdown");
                    if result.is_ok() {
                        result = Err(err);
                    }
                }
                Err(join_err) => {
                    tracing::error!(%join_err, "⛔ task join error during shutdown");
                    if result.is_ok() {
                        result = Err(Error::TaskJoin(join_err));
                    }
                }
            }
        }

        result
    }
}
