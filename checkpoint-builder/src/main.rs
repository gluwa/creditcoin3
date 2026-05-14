use anyhow::{bail, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

mod config;
mod sink;
mod source;

use config::{CheckpointConfig, CheckpointRange, Cli, SourceCommand};
use source::{ArchiveSource, RootSource, SledSource};

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file if it exists
    dotenvy::dotenv().ok();

    // Initialize tracing
    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();

    // Parse CLI arguments and build source + config
    let cli = Cli::parse();

    let (source, config): (Box<dyn RootSource>, CheckpointConfig) = match cli.command {
        SourceCommand::Sled(args) => {
            if !args.sled_db_path.exists() {
                bail!(
                    "Sled database path does not exist: {}",
                    args.sled_db_path.display()
                );
            }

            info!(
                "Using Sled database at path: {}",
                args.sled_db_path.display()
            );

            (
                Box::new(SledSource::open(&args.sled_db_path)?),
                CheckpointConfig::from_common(args.common)?,
            )
        }
        SourceCommand::Archiver(args) => (
            Box::new(ArchiveSource::new(args.archiver_url)?),
            CheckpointConfig::from_common(args.common)?,
        ),
    };

    if config.dry_run {
        let first = source
            .first()?
            .expect("Source should have at least one entry");
        let last = source
            .last()?
            .expect("Source should have at least one entry");
        println!("Available range: {} to {}", first.height, last.height);

        if config.validate_database {
            info!("Validating that all checkpoint ranges are present in the source...");
            if !config.validate_ranges(source.as_ref()) {
                bail!("Source validation failed: some checkpoint ranges are missing blocks. Please check the source and ensure all required blocks are present.");
            }
            info!("Source validation successful: all checkpoint ranges are present");
        }

        return Ok(());
    }

    info!("Starting checkpoint generator with config: \n{:#?}", config);

    // Log source info
    let first_block = source
        .first()?
        .expect("Source should have at least one entry");
    info!(
        "Source first entry at block {} with digest {}",
        first_block.height, first_block.digest
    );

    let last_block = source
        .last()?
        .expect("Source should have at least one entry");
    info!(
        "Source last entry at block {} with digest {}",
        last_block.height, last_block.digest
    );

    if config.starting_block() < first_block.height {
        bail!(
            "Starting block {} is before the first block in the source ({})",
            config.starting_block(),
            first_block.height
        );
    }

    if config.end_block() > last_block.height {
        bail!(
            "Ending block {} is after the last block in the source ({})",
            config.end_block(),
            last_block.height
        );
    }

    if config.validate_database {
        info!("Validating that all checkpoint ranges are present in the source...");
        if !config.validate_ranges(source.as_ref()) {
            bail!("Source validation failed: some checkpoint ranges are missing blocks. Please check the source and ensure all required blocks are present.");
        }
        info!("Source validation successful: all checkpoint ranges are present");
    } else {
        warn!("Source validation is disabled. If the source is missing any blocks in the specified ranges, checkpoint generation may fail or produce incomplete results.");
    }

    // Channel buffer is sized at 10x the commit interval to allow the producer to stay slightly ahead
    // of the consumer without blocking, enabling better throughput
    let commit_interval = config.checkpoint_commit_interval.get();
    let (tx, rx) = tokio::sync::mpsc::channel(commit_interval * 10);
    // Spawn background task to consume checkpoints and write to CSV
    let sink_handle =
        sink::csv::spawn_csv_sink(rx, config.output_file.clone(), commit_interval).await;

    // Run the main processing loop with CTRL-C handling
    let processing_result = tokio::select! {
        result = process_blocks(tx, source, &config) => result,
        _ = tokio::signal::ctrl_c() => {
            warn!("Received CTRL-C, initiating graceful shutdown...");
            Ok(())
        }
    };

    info!("Waiting for remaining checkpoints to be written to output file...");
    sink_handle.await??;

    if processing_result.is_ok() {
        info!("Checkpoint generation completed");
    }

    processing_result
}

async fn process_blocks(
    tx: mpsc::Sender<attestor_primitives::AttestationCheckpoint>,
    source: Box<dyn RootSource>,
    config: &CheckpointConfig,
) -> Result<()> {
    let starting_block = config.starting_block();

    // Initialize the checkpoint chain. This is the critical first step that determines
    // where the checkpoint chain begins.
    //
    // **Approach 1: Using Starting Digest (Resume from previous state)**
    // If --starting-digest is provided, it represents the checkpoint at block (starting_block - 1).
    // This allows resuming checkpoint generation after a previous run.
    // Example: If previous run ended at block 4999 with digest X, provide that digest
    // and start next run at block 5000.
    //
    // **Approach 2: Using Genesis Checkpoint (Start from new block)**
    // If the first range is a genesis checkpoint (e.g., "100"), we read that block from the
    // source and create the initial checkpoint from it. This is used when starting from
    // a known genesis block state.
    let mut latest_checkpoint = if let Some(digest) = config.starting_digest {
        // Case 1: Resume with provided digest
        tracing::info!(
            "Using provided starting digest as genesis checkpoint: {} at block {}",
            digest,
            starting_block - 1
        );

        attestor_primitives::AttestationCheckpoint {
            block_number: starting_block - 1,
            digest,
        }
    } else {
        // Case 2: Start with genesis checkpoint from source
        tracing::info!(
            "No starting digest provided, reading genesis block at height {} to create initial checkpoint",
            starting_block
        );

        // Read the genesis block from the source
        let genesis_block = source.get(starting_block)?.ok_or_else(|| {
            anyhow::anyhow!("Source has no block at genesis height {}", starting_block)
        })?;

        // Create checkpoint for the genesis block (no previous digest to chain from)
        let digest = attestor_primitives::compute_digest_for(
            genesis_block.height,
            &genesis_block.digest,
            None,
        );

        tracing::info!(
            "Read genesis block from source: {} at block {}",
            genesis_block.digest,
            genesis_block.height
        );

        let checkpoint = attestor_primitives::AttestationCheckpoint {
            block_number: genesis_block.height,
            digest,
        };

        // Send the genesis checkpoint to be written to output
        tx.send(checkpoint.clone()).await?;

        checkpoint
    };

    let ranges_len = config.ranges.len();

    // Process each range sequentially, updating the latest checkpoint as we go
    for (range_idx, range) in config.ranges.iter().enumerate() {
        match range {
            config::CheckpointRangeType::Genesis(_) => {
                // The genesis checkpoint is already handled above, so we don't process it here.
            }
            config::CheckpointRangeType::Regular(checkpoint_range) => {
                info!(
                    "Processing range {}/{}: blocks {} to {} with interval {}",
                    range_idx + 1,
                    ranges_len,
                    checkpoint_range.height_start,
                    checkpoint_range.height_end,
                    checkpoint_range.checkpoint_interval
                );

                latest_checkpoint =
                    process_range(&tx, source.as_ref(), checkpoint_range, latest_checkpoint)
                        .await?;
            }
        }
    }

    info!("Finished processing all {} ranges", config.ranges.len());

    Ok(())
}

/// Process a single checkpoint range, generating checkpoints at the specified interval.
///
/// This function iterates through blocks in the range [iteration_start, height_end] and
/// generates checkpoints every `checkpoint_interval` blocks. Each checkpoint is chained
/// to the previous one using keccak256(height || block_digest || prev_digest).
///
/// **Processing Steps:**
/// 1. Determines iteration start (skip already-processed blocks from genesis/previous ranges)
/// 2. Accumulates blocks in chunks of size `checkpoint_interval`
/// 3. Chains each chunk into a single checkpoint digest
/// 4. Sends completed checkpoints to CSV sink
/// 5. Handles partial final chunks with a warning (shouldn't happen if ranges were validated)
///
/// **Returns:** The final checkpoint state after processing this range
async fn process_range(
    tx: &mpsc::Sender<attestor_primitives::AttestationCheckpoint>,
    source: &dyn RootSource,
    range: &CheckpointRange,
    mut latest_checkpoint: attestor_primitives::AttestationCheckpoint,
) -> Result<attestor_primitives::AttestationCheckpoint> {
    // Determine the actual start block for iteration.
    // This handles resuming from a previous state by skipping blocks
    // that are already included in the latest checkpoint.
    let iteration_start = if latest_checkpoint.block_number >= range.height_start {
        // Skip blocks we've already processed in genesis or previous range
        latest_checkpoint.block_number + 1
    } else {
        // Start from the beginning of this range
        range.height_start
    };

    // If we're already past this range, skip it entirely
    if iteration_start > range.height_end {
        debug!(
            "Skipping range {} to {}: already processed (latest checkpoint at {})",
            range.height_start, range.height_end, latest_checkpoint.block_number
        );
        return Ok(latest_checkpoint);
    }

    let blocks_iter = source.iter_range(iteration_start, range.height_end);
    let mut current_chunk: Vec<source::RootInfo> = Vec::with_capacity(range.checkpoint_interval);

    for block_result in blocks_iter {
        let block = block_result?;

        current_chunk.push(block);

        // When we have a full chunk, process it into a checkpoint
        if current_chunk.len() == range.checkpoint_interval {
            debug!(
                "Processing block chunk of {} blocks (latest checkpoint at block {})",
                current_chunk.len(),
                latest_checkpoint.block_number
            );

            // Condense the chunk of blocks into a single checkpoint by progressively hashing them
            latest_checkpoint = current_chunk
                .drain(..)
                .fold(latest_checkpoint, |prev, block| {
                    attestor_primitives::AttestationCheckpoint {
                        block_number: block.height,
                        digest: attestor_primitives::compute_digest_for(
                            block.height,
                            &block.digest,
                            Some(&prev.digest),
                        ),
                    }
                });

            info!(
                "Sending checkpoint for block {} with digest {} to output file",
                latest_checkpoint.block_number, latest_checkpoint.digest
            );

            tx.send(latest_checkpoint.clone()).await?;
        }
    }

    // Handle any remaining blocks in the last partial chunk
    // This shouldn't happen if ranges are properly validated, but handle it gracefully
    if !current_chunk.is_empty() {
        warn!(
            "Processing unexpected partial chunk of {} blocks at end of range",
            current_chunk.len()
        );

        latest_checkpoint = current_chunk
            .drain(..)
            .fold(latest_checkpoint, |prev, block| {
                attestor_primitives::AttestationCheckpoint {
                    block_number: block.height,
                    digest: attestor_primitives::compute_digest_for(
                        block.height,
                        &block.digest,
                        Some(&prev.digest),
                    ),
                }
            });

        debug!(
            "Sending partial checkpoint for block {} with digest {} to output file",
            latest_checkpoint.block_number, latest_checkpoint.digest
        );

        tx.send(latest_checkpoint.clone()).await?;
    }

    Ok(latest_checkpoint)
}
