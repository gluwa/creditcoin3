//! Attestation monitoring module
//!
//! This module handles waiting for blocks to be attested on the Creditcoin3 network,
//! monitoring attestation events, and checking attestation status.

use anyhow::{Context, Result};
use cc_client::attestation::CcEvent;
use cc_client::Client as CcClient;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

/// Maximum reasonable block number (10 billion blocks)
/// Used to filter out invalid/corrupted checkpoint block numbers
/// This filters out corrupted data like 2163135196021391360
const MAX_REASONABLE_BLOCK: u64 = 10_000_000_000;

/// Configuration for attestation monitoring
#[derive(Debug, Clone)]
pub struct AttestationConfig {
    /// Maximum time to wait for attestation
    pub max_wait_time: Duration,
    /// Polling interval for checking attestation status
    pub poll_interval: Duration,
    /// Whether to check existing attestations before subscribing
    pub check_existing: bool,
}

impl Default for AttestationConfig {
    fn default() -> Self {
        Self {
            max_wait_time: Duration::from_secs(300), // 5 minutes
            poll_interval: Duration::from_secs(2),
            check_existing: true,
        }
    }
}

/// Result of attestation monitoring
#[derive(Debug, Clone)]
pub struct AttestationResult {
    /// The actual attested block number (may be higher than requested)
    pub attested_block: u64,
    /// How long it took to receive attestation
    pub wait_duration: Duration,
}

/// Monitor for attestations on the Creditcoin3 network
pub struct AttestationMonitor {
    cc3_client: CcClient,
    config: AttestationConfig,
}

impl AttestationMonitor {
    /// Create a new attestation monitor
    pub async fn new(cc3_rpc_url: &str, config: AttestationConfig) -> Result<Self> {
        let cc3_client = CcClient::new(cc3_rpc_url, "")
            .await
            .context("Failed to create Creditcoin3 client")?;

        Ok(Self { cc3_client, config })
    }

    /// Wait for a specific block to be attested using chain key directly
    pub async fn wait_for_block_attestation_with_chain_key(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<AttestationResult> {
        let start = std::time::Instant::now();

        info!(
            "Waiting for attestation of block {} on chain_key {}",
            block_number, chain_key
        );

        // First check if the block is already attested
        if self.config.check_existing {
            if let Some(result) = self
                .check_existing_attestation(chain_key, block_number)
                .await?
            {
                info!(
                    "Block {} already attested at height {}",
                    block_number, result.attested_block
                );
                return Ok(result);
            }
        }

        // Subscribe to attestation events
        self.monitor_attestation_events(chain_key, block_number, start)
            .await
    }

    /// Check if a block is already attested
    ///
    /// IMPORTANT: We check checkpoints FIRST before attestations to avoid a race condition.
    /// If a checkpoint exists for a block, the corresponding attestation may have been evicted
    /// from the retention buffer. By checking checkpoints first (which are permanent), we ensure
    /// we always find attested blocks even if attestations have been removed.
    async fn check_existing_attestation(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationResult>> {
        // Check checkpoints FIRST - they're permanent and won't be evicted
        // Note: checkpoint.block_number refers to the Ethereum block that was checkpointed.
        // A checkpoint at block X means that block X (and all previous blocks) have been checkpointed.
        let checkpoints = self
            .cc3_client
            .get_checkpoints_for_chain(chain_key)
            .await
            .unwrap_or_default();

        for checkpoint in checkpoints {
            // Validate checkpoint block number is reasonable
            if checkpoint.block_number > MAX_REASONABLE_BLOCK {
                debug!(
                    "Skipping invalid checkpoint with block number {} (too large, likely corrupted)",
                    checkpoint.block_number
                );
                continue;
            }

            // A checkpoint at block X means block X and all previous blocks are checkpointed
            if checkpoint.block_number >= block_number {
                return Ok(Some(AttestationResult {
                    attested_block: checkpoint.block_number,
                    wait_duration: Duration::from_secs(0),
                }));
            }
        }

        // Then check attestations (may be evicted after checkpoint creation)
        let attestations = self
            .cc3_client
            .get_attestations_for_chain(chain_key)
            .await
            .unwrap_or_default();

        for attestation in attestations {
            if attestation.attestation.header_number >= block_number {
                return Ok(Some(AttestationResult {
                    attested_block: attestation.attestation.header_number,
                    wait_duration: Duration::from_secs(0),
                }));
            }
        }

        Ok(None)
    }

    /// Monitor attestation events until the block is attested
    async fn monitor_attestation_events(
        &self,
        chain_key: u64,
        block_number: u64,
        start_time: std::time::Instant,
    ) -> Result<AttestationResult> {
        let mut subscription = self
            .cc3_client
            .subscribe_events(chain_key)
            .context("Failed to subscribe to attestation events")?;

        info!(
            "Subscribed to attestation events for chain_key {}",
            chain_key
        );

        loop {
            // Check timeout
            let elapsed = start_time.elapsed();
            if elapsed > self.config.max_wait_time {
                subscription.cancel()?;
                return Err(anyhow::anyhow!(
                    "Timeout waiting for attestation of block {block_number} (waited {elapsed:?})"
                ));
            }

            // Wait for next event with timeout
            match tokio::time::timeout(Duration::from_secs(5), subscription.next()).await {
                Ok(Some(event)) => {
                    if let Some(result) =
                        self.process_event(event, chain_key, block_number, elapsed)?
                    {
                        subscription.cancel()?;
                        return Ok(result);
                    }
                }
                Ok(None) => {
                    debug!("No attestation event received, subscription may have ended");
                    // Resubscribe if needed
                    subscription = self
                        .cc3_client
                        .subscribe_events(chain_key)
                        .context("Failed to resubscribe to attestation events")?;
                }
                Err(_) => {
                    debug!(
                        "Event timeout, checking existing attestations (elapsed: {:?})",
                        elapsed
                    );

                    // Periodically check if the block was attested while we were waiting
                    if let Some(mut result) = self
                        .check_existing_attestation(chain_key, block_number)
                        .await?
                    {
                        result.wait_duration = elapsed;
                        subscription.cancel()?;
                        return Ok(result);
                    }
                }
            }

            // Small delay before next iteration
            sleep(self.config.poll_interval).await;
        }
    }

    /// Process a single attestation event
    fn process_event(
        &self,
        event: CcEvent,
        chain_key: u64,
        block_number: u64,
        elapsed: Duration,
    ) -> Result<Option<AttestationResult>> {
        match event {
            CcEvent::BlockAttested(metadata) => {
                let attested_block = metadata.header_number();
                let attested_chain = metadata.chain_key();

                debug!(
                    "Received BlockAttested event for block {} on chain_key {}",
                    attested_block, attested_chain
                );

                if attested_chain == chain_key && attested_block >= block_number {
                    info!(
                        "Block {} attested (attestation for block {}, elapsed: {:?})",
                        block_number, attested_block, elapsed
                    );

                    return Ok(Some(AttestationResult {
                        attested_block,
                        wait_duration: elapsed,
                    }));
                }
            }
            CcEvent::CheckpointReached(checkpoint) => {
                debug!(
                    "Received CheckpointReached event for block {} on chain_key {}",
                    checkpoint.block_number, chain_key
                );

                // Validate checkpoint block number is reasonable before using it
                if checkpoint.block_number <= MAX_REASONABLE_BLOCK
                    && checkpoint.block_number >= block_number
                {
                    info!(
                        "Checkpoint reached at block {}, covers our block {} (elapsed: {:?})",
                        checkpoint.block_number, block_number, elapsed
                    );

                    return Ok(Some(AttestationResult {
                        attested_block: checkpoint.block_number,
                        wait_duration: elapsed,
                    }));
                } else if checkpoint.block_number > MAX_REASONABLE_BLOCK {
                    debug!(
                        "Ignoring invalid checkpoint with block number {} (too large, likely corrupted)",
                        checkpoint.block_number
                    );
                }
            }
            _ => {
                // Ignore other events
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_attestation_config_default() {
        let config = AttestationConfig::default();
        assert_eq!(config.max_wait_time, Duration::from_secs(300));
        assert_eq!(config.poll_interval, Duration::from_secs(2));
        assert!(config.check_existing);
    }

    #[tokio::test]
    async fn test_attestation_result() {
        let result = AttestationResult {
            attested_block: 105,
            wait_duration: Duration::from_secs(30),
        };

        assert_eq!(result.attested_block, 105);
        assert_eq!(result.wait_duration, Duration::from_secs(30));
    }
}
