use anyhow::Result;
use cc_client::{attestation::CcEvent, Client as CcClient};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

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
/// Used to quickly determine if a query needs checkpoint data
pub type LastCheckpointBlockMap = Arc<RwLock<std::collections::HashMap<u64, u64>>>;

/// Get the last attestation for a chain (for local view)
pub fn get_last_attestation(chain_key: u64) -> Option<LastAttestation> {
    LAST_ATTESTATIONS
        .lock()
        .ok()
        .and_then(|cache| cache.iter().find(|a| a.chain_key == chain_key).cloned())
}

/// Start CC3 event subscription with automatic reconnection on failure.
///
/// This function will continuously attempt to maintain a subscription to CC3 events,
/// reconnecting with exponential backoff if the connection is lost. Events are logged
/// and cached in-memory for local view.
///
/// Also monitors for checkpoint interval changes and updates the provided map.
/// Also tracks the last checkpoint block number per chain and updates the builder.
pub async fn start_cc3_event_subscription(
    cc3_client: Arc<CcClient>,
    chain_key: u64,
    checkpoint_intervals: CheckpointIntervalMap,
    last_checkpoint_blocks: LastCheckpointBlockMap,
    builder: std::sync::Arc<continuity::ContinuityBuilder>,
) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;

    loop {
        info!("Starting CC3 event subscription for chain_key: {chain_key}");

        match cc3_client.subscribe_events(chain_key).await {
            Ok(mut subscription) => {
                info!("Successfully subscribed to CC3 events for chain_key: {chain_key}");
                let mut received_event = false;

                loop {
                    match subscription.next().await {
                        Some(cc_event) => {
                            // Reset backoff after successfully receiving at least one event,
                            // not just on connection success. This prevents tight reconnect
                            // loops when the server accepts connections but immediately drops them.
                            if !received_event {
                                received_event = true;
                                backoff = INITIAL_BACKOFF;
                            }

                            if let Err(e) = process_cc_event(
                                &cc_event,
                                chain_key,
                                &checkpoint_intervals,
                                &last_checkpoint_blocks,
                                &builder,
                            )
                            .await
                            {
                                error!("Failed to process CC3 event: {e}");
                                // Continue processing other events even if one fails
                            }
                        }
                        None => {
                            warn!(
                                "CC3 event subscription ended unexpectedly, reconnecting in {backoff:?}"
                            );
                            break; // Break inner loop to reconnect
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to subscribe to CC3 events: {e}, retrying in {backoff:?}");
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Process a CC3 event, filtering by chain_key and caching attestations in-memory.
async fn process_cc_event(
    event: &CcEvent,
    chain_key: u64,
    checkpoint_intervals: &CheckpointIntervalMap,
    last_checkpoint_blocks: &LastCheckpointBlockMap,
    builder: &std::sync::Arc<continuity::ContinuityBuilder>,
) -> Result<()> {
    match event {
        CcEvent::BlockAttested(metadata) => {
            if metadata.chain_key != chain_key {
                return Ok(()); // Not our chain, ignore
            }

            let header_number = metadata.header_number;
            let digest = metadata.digest;
            info!(
                "Processing BlockAttested event: chain_key={chain_key}, header_number={header_number}, digest={digest:?}"
            );

            // Cache in-memory for local view
            if let Ok(mut cache) = LAST_ATTESTATIONS.lock() {
                // Update or insert last attestation for this chain
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

            info!("Cached attestation in-memory: chain_key={chain_key}, header_number={header_number}");

            Ok(())
        }
        CcEvent::CheckpointReached(checkpoint) => {
            let block_number = checkpoint.block_number;
            let digest = checkpoint.digest;
            info!(
                "Processing CheckpointReached event: chain_key={chain_key}, block_number={block_number}, digest={digest:?}"
            );

            // Update last checkpoint block number for this chain
            let mut checkpoint_blocks = last_checkpoint_blocks.write().await;
            let current_last = checkpoint_blocks.get(&chain_key).copied().unwrap_or(0);
            if block_number > current_last {
                checkpoint_blocks.insert(chain_key, block_number);
                info!(
                    "✅ Updated last checkpoint block for chain {chain_key} to {}",
                    block_number
                );

                // Also update the builder's last checkpoint block for optimization
                builder.update_last_checkpoint_block(block_number).await;
            }

            Ok(())
        }
        CcEvent::CheckpointIntervalChanged(new_interval) => {
            let interval_u64 = *new_interval;
            info!(
                "⚙️  Checkpoint interval changed for chain {chain_key}: {} attestations",
                interval_u64
            );

            // Update the checkpoint interval map
            let mut intervals = checkpoint_intervals.write().await;
            intervals.insert(chain_key, interval_u64);

            info!(
                "✅ Updated checkpoint interval for chain {chain_key} to {}",
                interval_u64
            );

            Ok(())
        }
        _ => Ok(()), // Ignore other event types
    }
}
