use anyhow::Result;
use cc_client::{events::CcEvent, Client as CcClient};
use futures::TryStreamExt;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::ContinuityService;

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
/// Will automatically reconnect to the CC3 node if the connection is lost.
pub async fn start_cc3_event_subscription(
    cc3_client: CcClient,
    checkpoint_intervals: CheckpointIntervalMap,
    last_checkpoint_blocks: LastCheckpointBlockMap,
    service: Arc<ContinuityService>,
) -> Result<()> {
    let chain_keys = service.configured_chain_keys();

    anyhow::ensure!(
        !chain_keys.is_empty(),
        "no chains configured for event subscription"
    );

    let config = stream::cc3::ConfigBuilder::new()
        .with_cc3(cc3_client.clone())
        .with_chain_keys(chain_keys.iter().copied().collect::<Vec<_>>())
        .build();
    let mut events = stream::cc3::StreamCC3::new(config).await?.try_flatten();

    loop {
        match events.try_next().await {
            Ok(Some(cc_event)) => {
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
            Ok(None) => {
                anyhow::bail!("End of unbounded event stream");
            }
            Err(e) => {
                error!("❌ 🔗 Failed to retrieve CC3 event: {e}")
            }
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
            service.update_checkpoint_interval(*event_chain_key, interval_u64);

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
