use anyhow::Result;
use cc_client::{attestation::CcEvent, Client as CcClient};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::ContinuityService;

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

    let mut backoff = INITIAL_BACKOFF;

    loop {
        info!(
            ?chain_keys,
            "Starting CC3 event subscription (single task, multi-chain filter)"
        );

        match cc3_client.subscribe_events_chains(chain_keys.clone()) {
            Ok(mut subscription) => {
                info!(
                    "Successfully subscribed to CC3 events for chain keys: {:?}",
                    chain_keys
                );
                let mut received_event = false;

                loop {
                    match subscription.next().await {
                        Some(cc_event) => {
                            if !received_event {
                                received_event = true;
                                backoff = INITIAL_BACKOFF;
                            }

                            if let Err(e) = process_cc_event(
                                &cc_event,
                                &checkpoint_intervals,
                                &last_checkpoint_blocks,
                                &service,
                            )
                            .await
                            {
                                error!("Failed to process CC3 event: {e}");
                            }
                        }
                        None => {
                            warn!(
                                "CC3 event subscription ended unexpectedly, reconnecting in {backoff:?}"
                            );
                            break;
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
                "Processing BlockAttested event: chain_key={chain_key}, header_number={header_number}, digest={digest:?}"
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

            info!("Cached attestation in-memory: chain_key={chain_key}, header_number={header_number}");

            Ok(())
        }
        CcEvent::CheckpointReached(event_chain_key, checkpoint) => {
            if !service.serves_chain(*event_chain_key) {
                return Ok(());
            }

            let block_number = checkpoint.block_number;
            let digest = checkpoint.digest;
            info!(
                "Processing CheckpointReached event: chain_key={event_chain_key}, block_number={block_number}, digest={digest:?}"
            );

            let mut checkpoint_blocks = last_checkpoint_blocks.write().await;
            let current_last = checkpoint_blocks.get(event_chain_key).copied().unwrap_or(0);
            if block_number > current_last {
                checkpoint_blocks.insert(*event_chain_key, block_number);
                info!(
                    "✅ Updated last checkpoint block for chain {event_chain_key} to {}",
                    block_number
                );

                service
                    .update_builder_last_checkpoint(*event_chain_key, block_number)
                    .await;
            }

            service
                .insert_checkpoint(*event_chain_key, block_number, digest)
                .await;

            Ok(())
        }
        CcEvent::CheckpointIntervalChanged(event_chain_key, new_interval) => {
            if !service.serves_chain(*event_chain_key) {
                return Ok(());
            }
            let interval_u64 = *new_interval;
            info!(
                "⚙️  Checkpoint interval changed for chain {event_chain_key}: {} attestations",
                interval_u64
            );

            let mut intervals = checkpoint_intervals.write().await;
            intervals.insert(*event_chain_key, interval_u64);

            info!(
                "✅ Updated checkpoint interval for chain {event_chain_key} to {}",
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
                "⚙️  Chain genesis block changed for chain {event_chain_key}: {}",
                new_genesis
            );

            service
                .update_genesis_block(*event_chain_key, new_genesis)
                .await;

            info!(
                "✅ Updated chain genesis block for chain {event_chain_key} to {}",
                new_genesis
            );

            Ok(())
        }
        _ => Ok(()),
    }
}
