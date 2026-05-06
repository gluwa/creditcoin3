use anyhow::Result;
use cc_client::{attestation::CcEvent, Client as CcClient};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::ContinuityService;

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// If the subscription stayed connected for at least this long before disconnecting,
/// treat the failure as transient and reset the reconnect backoff. Anything shorter
/// is treated as a hard failure (e.g. wire-protocol mismatch) and we keep the
/// exponential backoff so we don't hammer the RPC endpoint.
const HEALTHY_CONNECTION_THRESHOLD: Duration = Duration::from_secs(30);

/// Simple in-memory cache for last attestation per chain
#[derive(Clone, Debug)]
pub struct LastAttestation {
    pub chain_key: u64,
    pub header_number: u64,
    pub digest: sp_core::H256,
}

/// Global in-memory cache for last attestations
static LAST_ATTESTATIONS: Mutex<Vec<LastAttestation>> = Mutex::new(Vec::new());

/// Checkpoint interval per chain (stored in Arc<RwLock> for dynamic updates)
pub type CheckpointIntervalMap = Arc<RwLock<std::collections::HashMap<u64, u64>>>;

/// Last checkpoint block number per chain (stored in Arc<RwLock> for dynamic updates)
pub type LastCheckpointBlockMap = Arc<RwLock<std::collections::HashMap<u64, u64>>>;

/// Get the last attestation for a chain (for local view)
pub fn get_last_attestation(chain_key: u64) -> Option<LastAttestation> {
    LAST_ATTESTATIONS
        .lock()
        .ok()
        .and_then(|cache| cache.iter().find(|a| a.chain_key == chain_key).cloned())
}

/// Start a single CC3 event subscription for all configured chain keys (one finalized-block stream).
pub async fn start_cc3_event_subscription(
    cc3_client: Arc<CcClient>,
    checkpoint_intervals: CheckpointIntervalMap,
    last_checkpoint_blocks: LastCheckpointBlockMap,
    service: Arc<ContinuityService>,
) -> Result<()> {
    let chain_keys: HashSet<u64> = service.configured_chain_keys();
    if chain_keys.is_empty() {
        anyhow::bail!("no chains configured for event subscription");
    }

    let chain_keys_vec: Vec<u64> = chain_keys.iter().copied().collect();

    // We hold the *same* `Arc<CcClient>` shared with every `ContinuityBuilder`
    // for the entire loop. `CcClient` keeps its live subxt RPC + OnlineClient
    // behind an `ArcSwap<ClientInner>`, so calling `reconnect()` here
    // atomically swaps the underlying connection for every Arc holder at
    // once. The previous design value-cloned the client, reconnected the
    // clone, and dropped the original — which left every other Arc holder
    // (every `ContinuityBuilder`) pinned to the dead subxt connection.

    let mut backoff = INITIAL_BACKOFF;
    let mut consecutive_failures: u32 = 0;

    loop {
        info!(
            ?chain_keys,
            attempt = consecutive_failures + 1,
            "🔗 Starting CC3 event subscription (single task, multi-chain filter)"
        );

        let connected_at = Instant::now();

        match cc3_client.subscribe_events_chains(&chain_keys_vec) {
            Ok(mut subscription) => {
                info!(
                    "🔗 Successfully subscribed to CC3 events for chain keys: {:?}",
                    chain_keys
                );

                loop {
                    match subscription.next().await {
                        Some(cc_event) => {
                            if let Err(e) = process_cc_event(
                                &cc_event,
                                &checkpoint_intervals,
                                &last_checkpoint_blocks,
                                &service,
                            )
                            .await
                            {
                                error!("❌ 🔗 Failed to process CC3 event: {e}");
                            }
                        }
                        None => {
                            // Stream ended. Pull the real cause out of the
                            // spawned task instead of just logging "ended
                            // unexpectedly" with no context.
                            let connection_duration = connected_at.elapsed();
                            match subscription.take_terminal_error().await {
                                Ok(Some(err)) => {
                                    error!(
                                        connection_duration_secs =
                                            connection_duration.as_secs_f64(),
                                        "❌ 🔗 CC3 event subscription terminated with error: {err:?}"
                                    );
                                }
                                Ok(None) => {
                                    warn!(
                                        connection_duration_secs = connection_duration.as_secs_f64(),
                                        "⚠️ 🔗 CC3 event subscription closed cleanly by backend (no error reported)"
                                    );
                                }
                                Err(join_err) => {
                                    error!(
                                        connection_duration_secs =
                                            connection_duration.as_secs_f64(),
                                        "💥 🔗 CC3 event subscription task panicked: {join_err:?}"
                                    );
                                }
                            }
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    attempt = consecutive_failures + 1,
                    "❌ 🔗 Failed to subscribe to CC3 events: {e:?}"
                );
            }
        }

        // Decide how to back off based on how long the connection survived.
        // If we stayed up for a healthy window before dropping, the failure
        // looks transient — reset to a fast retry. Otherwise keep growing the
        // backoff exponentially so a hard misconfiguration (e.g. wire-protocol
        // mismatch with the RPC) doesn't spin in a tight loop.
        let connection_duration = connected_at.elapsed();
        if connection_duration >= HEALTHY_CONNECTION_THRESHOLD {
            if backoff != INITIAL_BACKOFF {
                info!(
                    connection_duration_secs = connection_duration.as_secs_f64(),
                    "🔄 🔗 Resetting CC3 reconnect backoff after a healthy connection"
                );
            }
            backoff = INITIAL_BACKOFF;
            consecutive_failures = 0;
        } else {
            consecutive_failures = consecutive_failures.saturating_add(1);
        }

        warn!(
            backoff_secs = backoff.as_secs_f64(),
            consecutive_failures, "🔄 🔗 Reconnecting CC3 event subscription"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);

        // Rebuild the underlying subxt RPC + OnlineClient before the next
        // subscribe attempt. After a clean WebSocket close (e.g. the CC3 node
        // restarted) the subxt client transitions to a `RestartNeeded` state
        // and every future call returns `RestartNeeded(Transport(Connection(
        // Closed)))` — calling `subscribe_finalized` again on the same client
        // would never recover. Refreshing it here is what actually breaks the
        // tight reconnect loop seen on node restart.
        //
        // Because the `Arc<CcClient>` is shared with every continuity builder
        // and the `CcClient` stores its inner connection behind `ArcSwap`,
        // this single `reconnect()` call refreshes the live subxt handle for
        // every consumer in the process at once.
        //
        // If `reconnect` fails (the node is still down), we keep going: the
        // next `subscribe_events_chains` call will fail with the same error,
        // surface that via the existing logging path, and we'll back off and
        // retry until the node comes back.
        info!("🔄 🔗 Refreshing CC3 RPC connection before next subscribe attempt");
        match cc3_client.reconnect().await {
            Ok(()) => info!("✅ 🔗 CC3 RPC connection refreshed"),
            Err(err) => warn!(
                ?err,
                "⚠️ 🔗 Failed to refresh CC3 RPC connection; will retry on next loop iteration"
            ),
        }
    }
}

async fn process_cc_event(
    event: &CcEvent,
    checkpoint_intervals: &CheckpointIntervalMap,
    last_checkpoint_blocks: &LastCheckpointBlockMap,
    service: &Arc<ContinuityService>,
) -> Result<()> {
    match event {
        CcEvent::BlockAttested(metadata) => {
            let chain_key = metadata.chain_key;
            if !service.serves_chain(chain_key) {
                return Ok(());
            }

            let header_number = metadata.header_number;
            let digest = metadata.digest;
            info!(
                "🔗 📜 Processing BlockAttested event: chain_key={chain_key}, header_number={header_number}, digest={digest:?}"
            );

            if let Ok(mut cache) = LAST_ATTESTATIONS.lock() {
                if let Some(existing) = cache.iter_mut().find(|a| a.chain_key == chain_key) {
                    *existing = LastAttestation {
                        chain_key,
                        header_number,
                        digest,
                    };
                } else {
                    cache.push(LastAttestation {
                        chain_key,
                        header_number,
                        digest,
                    });
                }
            }

            service
                .insert_attestation(chain_key, header_number, digest)
                .await;

            info!("🔗 📦 Cached attestation in-memory: chain_key={chain_key}, header_number={header_number}");

            Ok(())
        }
        CcEvent::CheckpointReached(event_chain_key, checkpoint) => {
            if !service.serves_chain(*event_chain_key) {
                return Ok(());
            }

            let block_number = checkpoint.block_number;
            let digest = checkpoint.digest;
            info!(
                "🔗 🚩 Processing CheckpointReached event: chain_key={event_chain_key}, block_number={block_number}, digest={digest:?}"
            );

            let mut checkpoint_blocks = last_checkpoint_blocks.write().await;
            let current_last = checkpoint_blocks.get(event_chain_key).copied().unwrap_or(0);
            if block_number > current_last {
                checkpoint_blocks.insert(*event_chain_key, block_number);
                info!(
                    "🔗 ✅ Updated last checkpoint block for chain {event_chain_key} to {}",
                    block_number
                );

                service
                    .update_builder_last_checkpoint(*event_chain_key, block_number)
                    .await;
            }

            service
                .insert_checkpoint(*event_chain_key, block_number, digest)
                .await;

            // Mirror the on-chain pallet's `remove_attestations` behavior:
            // attestations consumed by this checkpoint are queued for
            // eviction (and dropped once the per-chain retention window is
            // exceeded). Keeping them in cache risks anchoring future proofs
            // on digests the on-chain verifier has already discarded.
            service
                .prune_attestations_at_or_below(*event_chain_key, block_number)
                .await;

            Ok(())
        }
        CcEvent::CheckpointIntervalChanged(event_chain_key, new_interval) => {
            if !service.serves_chain(*event_chain_key) {
                return Ok(());
            }
            let interval_u64 = *new_interval;
            info!(
                "🔗 ⚙️  Checkpoint interval changed for chain {event_chain_key}: {} attestations",
                interval_u64
            );

            let mut intervals = checkpoint_intervals.write().await;
            intervals.insert(*event_chain_key, interval_u64);

            info!(
                "🔗 ✅ Updated checkpoint interval for chain {event_chain_key} to {}",
                interval_u64
            );

            Ok(())
        }
        CcEvent::AttestationChainGenesisBlockNumberSet(event_chain_key, new_genesis) => {
            if !service.serves_chain(*event_chain_key) {
                return Ok(());
            }
            let new_genesis = *new_genesis;
            info!(
                "🔗 ⚙️  Chain genesis block changed for chain {event_chain_key}: {}",
                new_genesis
            );

            service
                .update_genesis_block(*event_chain_key, new_genesis)
                .await;

            info!(
                "🔗 ✅ Updated chain genesis block for chain {event_chain_key} to {}",
                new_genesis
            );

            Ok(())
        }
        CcEvent::RevertedAttestationChainTo(event_chain_key, checkpoint_height, _digest) => {
            let chain_key = *event_chain_key;
            if !service.serves_chain(chain_key) {
                return Ok(());
            }

            let revert_height = *checkpoint_height;
            warn!(
                "🔗 ⚠️  Attestation chain reversion detected for chain {chain_key}: \
                 reverting caches to height {revert_height}"
            );

            // Truncate per chain attestation and checkpoint boundary caches
            service.revert_caches(chain_key, revert_height).await;

            // Clear last seen entries. they'll be repopulated by the next
            // BlockAttested / CheckpointReached events.
            if let Ok(mut cache) = LAST_ATTESTATIONS.lock() {
                cache.retain(|a| a.chain_key != chain_key);
            }
            last_checkpoint_blocks.write().await.remove(&chain_key);

            info!(
                "🔗 ✅ Attestation chain reversion handled for chain {chain_key} \
                 (reverted to height {revert_height})"
            );

            Ok(())
        }
        _ => Ok(()),
    }
}
