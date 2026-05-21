//! Attestation monitoring module
//!
//! This module handles waiting for blocks to be attested on the Creditcoin3 network,
//! monitoring attestation events, and checking attestation status.

use anyhow::{Context, Result};
use cc_client::events::CcEvent;
use cc_client::Client as CcClient;
use futures::{StreamExt, TryStreamExt};
use std::time::Duration;
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
    /// Whether to check existing attestations before subscribing
    pub check_existing: bool,
}

impl Default for AttestationConfig {
    fn default() -> Self {
        Self {
            max_wait_time: Duration::from_secs(300), // 5 minutes
            check_existing: true,
        }
    }
}

/// Result of attestation monitoring
#[derive(Debug, Clone)]
pub struct AttestationResult {
    /// The actual attested block number (may be higher than requested)
    pub attested_block: u64,
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
        let config = stream::cc3::ConfigBuilder::new()
            .with_cc3(self.cc3_client.clone())
            .with_chain_keys(vec![chain_key])
            .build();
        let mut stream_cc3 = stream::cc3::StreamCC3::new(config)
            .await
            .context("Failed to subscribe to cc3 events")?;

        info!(
            "Subscribed to attestation events for chain_key {}",
            chain_key
        );

        let timeout = tokio::time::sleep_until(
            start_time
                .checked_add(self.config.max_wait_time)
                .unwrap_or(start_time)
                .into(),
        );
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                Some(res) = stream_cc3.next() => {
                    let elapsed = start_time.elapsed();
                    let mut events = res?;

                    while let Some(event) = events.try_next().await? {
                        if let Some(result) =
                            self.process_event(event, chain_key, block_number, elapsed)?
                        {
                            return Ok(result);
                        }
                    }
                }
                _ = &mut timeout => {
                    let elapsed = self.config.max_wait_time;
                    return Err(anyhow::anyhow!(
                        "Timeout waiting for attestation of block {block_number} (waited {elapsed:?})"
                    ));
                }
            }
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

                    return Ok(Some(AttestationResult { attested_block }));
                }
            }
            CcEvent::CheckpointReached(event_chain_key, checkpoint) => {
                debug!(
                    "Received CheckpointReached event for block {} on chain_key {}",
                    checkpoint.block_number, event_chain_key
                );

                if event_chain_key != chain_key {
                    return Ok(None);
                }

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
        assert!(config.check_existing);
    }

    #[tokio::test]
    async fn test_attestation_result() {
        let result = AttestationResult {
            attested_block: 105,
        };

        assert_eq!(result.attested_block, 105);
    }
}
