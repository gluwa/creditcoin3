use anyhow::{Context, Result};
use cc_client::{attestation::CcEvent, Client as CcClient};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::db::continuity_proofs::ContinuityProofItem;
use crate::db::{continuity_blocks, models::ContinuityBlockItem, DbManager};
use crate::indexer::IndexerClient;

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

// Retry settings for indexer proof fetching
const INDEXER_FETCH_MAX_RETRIES: u32 = 3;
const INDEXER_FETCH_INITIAL_DELAY: Duration = Duration::from_secs(2);

/// Start CC3 event subscription with automatic reconnection on failure.
///
/// This function will continuously attempt to maintain a subscription to CC3 events,
/// reconnecting with exponential backoff if the connection is lost. Events are processed
/// directly and stored to the database.
///
/// If an `indexer_client` is provided, continuity proofs will be pre-fetched from the
/// indexer when `BlockAttested` events are received.
pub async fn start_cc3_event_subscription(
    cc3_client: Arc<CcClient>,
    db_manager: Arc<DbManager>,
    chain_key: u64,
    indexer_client: Option<Arc<IndexerClient>>,
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
                                &db_manager,
                                indexer_client.as_ref(),
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

/// Process a CC3 event, filtering by chain_key and storing relevant events.
async fn process_cc_event(
    event: &CcEvent,
    chain_key: u64,
    db_manager: &Arc<DbManager>,
    indexer_client: Option<&Arc<IndexerClient>>,
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

            let digest_str = format!("0x{digest:x}");
            let attestation_item =
                ContinuityBlockItem::from_attestation(chain_key, header_number, digest_str);

            let mut conn = db_manager
                .pool()
                .get()
                .await
                .context("Failed to get database connection")?;

            continuity_blocks::insert_attestation(&mut conn, &attestation_item)
                .await
                .context("Failed to store attestation")?;

            info!("Successfully stored attestation: chain_key={chain_key}, header_number={header_number}");

            // Spawn fire-and-forget task to fetch continuity proof from indexer
            if let Some(client) = indexer_client {
                let client = Arc::clone(client);
                let db = Arc::clone(db_manager);
                tokio::spawn(async move {
                    fetch_and_store_continuity_proof(client, db, chain_key, header_number).await;
                });
            }

            Ok(())
        }
        CcEvent::CheckpointReached(checkpoint) => {
            // CheckpointReached events are chain-specific (no chain_key in the event)
            // We process all checkpoints since we're subscribed to a specific chain
            let block_number = checkpoint.block_number;
            let digest = checkpoint.digest;
            info!(
                "Processing CheckpointReached event: chain_key={chain_key}, block_number={block_number}, digest={digest:?}"
            );

            let digest_str = format!("0x{digest:x}");

            let mut conn = db_manager
                .pool()
                .get()
                .await
                .context("Failed to get database connection")?;

            // Use upsert to update existing attestation to checkpoint, or insert if not exists
            continuity_blocks::upsert_checkpoint(&mut conn, chain_key, block_number, &digest_str)
                .await
                .context("Failed to upsert checkpoint")?;

            info!("Successfully upserted checkpoint: chain_key={chain_key}, block_number={block_number}");

            Ok(())
        }
        _ => Ok(()), // Ignore other event types
    }
}

/// Fetch continuity proof from the indexer and store it in the database.
///
/// This function is called as a fire-and-forget background task when a BlockAttested
/// event is received. It retries with exponential backoff since the indexer may not
/// have indexed the attestation yet.
async fn fetch_and_store_continuity_proof(
    indexer_client: Arc<IndexerClient>,
    db_manager: Arc<DbManager>,
    chain_key: u64,
    header_number: u64,
) {
    let mut delay = INDEXER_FETCH_INITIAL_DELAY;

    for attempt in 1..=INDEXER_FETCH_MAX_RETRIES {
        match indexer_client
            .get_continuity_proof(chain_key, header_number)
            .await
        {
            Ok(Some(proof)) => {
                // Store the proof in the database
                let proof_item = ContinuityProofItem {
                    chain_key,
                    header_number,
                    continuity_proof: proof,
                    ends_in_attestation: true, // BlockAttested events always end in attestation
                };
                db_manager.insert_continuity_proof(proof_item);
                info!(
                    "Pre-fetched continuity proof from indexer: chain_key={}, header_number={}",
                    chain_key, header_number
                );
                return;
            }
            Ok(None) => {
                // Attestation not yet indexed, retry after delay
                if attempt < INDEXER_FETCH_MAX_RETRIES {
                    debug!(
                        "Attestation not yet in indexer (attempt {}/{}), retrying in {:?}: chain_key={}, header_number={}",
                        attempt, INDEXER_FETCH_MAX_RETRIES, delay, chain_key, header_number
                    );
                    tokio::time::sleep(delay).await;
                    delay *= 2; // Exponential backoff: 2s -> 4s -> 8s
                }
            }
            Err(e) => {
                // Network or parse error, retry after delay
                if attempt < INDEXER_FETCH_MAX_RETRIES {
                    warn!(
                        "Failed to fetch proof from indexer (attempt {}/{}), retrying in {:?}: chain_key={}, header_number={}, error={}",
                        attempt, INDEXER_FETCH_MAX_RETRIES, delay, chain_key, header_number, e
                    );
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                } else {
                    warn!(
                        "Failed to fetch proof from indexer after {} attempts: chain_key={}, header_number={}, error={}",
                        INDEXER_FETCH_MAX_RETRIES, chain_key, header_number, e
                    );
                }
            }
        }
    }

    // All retries exhausted without finding the proof
    debug!(
        "Could not pre-fetch continuity proof from indexer after {} attempts: chain_key={}, header_number={}",
        INDEXER_FETCH_MAX_RETRIES, chain_key, header_number
    );
}
