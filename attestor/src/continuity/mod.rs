use anyhow::Result;
use sp_core::H256;
use std::collections::BTreeMap;
use thiserror::Error;
use tracing::{debug, error, warn};

use attestation_chain::{block::Block, continuity_chain::Manager};
use eth::Client;

pub use attestation_chain::{
    attestation_fragment::{AttestationFragment, AttestationFragmentSerializable},
    continuity_chain::{CreateResult, Error as ContinuityError},
};

#[derive(Debug, Clone)]
pub struct Cache {
    // Add fields here
    eth_client: Client,
    // Block cache
    blocks: BTreeMap<u64, Block>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to create fragment, invalid parameters: start {0}, end {1}")]
    InvalidParameters(u64, u64),
    #[error("Continuity error: {0}")]
    ContinuityError(#[from] ContinuityError),
}

impl Cache {
    /// Creates a new cache instance.
    pub fn new(eth_client: Client) -> Self {
        Cache {
            eth_client,
            blocks: BTreeMap::new(),
        }
    }

    /// Prunes cache by removing blocks older than the specified block number.
    pub fn prune_all_before(&mut self, block_number: u64) {
        // Remove all blocks before the specified block number
        debug!("Pruning cache. Removing blocks older than {}", block_number);
        self.blocks.retain(|&k, _| k >= block_number);
    }

    /// Creates a new fragment from the specified block range.
    pub async fn async_retry_create(
        &mut self,
        start_block: u64,
        from_digest: H256,
        end_block: u64,
    ) -> Result<AttestationFragment, Error> {
        if start_block > end_block {
            return Err(Error::InvalidParameters(start_block, end_block));
        }

        debug!(
            "Creating fragment from block {} to {}",
            start_block, end_block
        );

        debug!("Cached blocks len: {}", self.blocks.len());

        let mut missing_ranges = Vec::new();
        let mut range_start = None;

        for block_number in start_block..=end_block {
            if !self.blocks.contains_key(&block_number) {
                if range_start.is_none() {
                    range_start = Some(block_number);
                }
            } else if let Some(start) = range_start.take() {
                missing_ranges.push((start, block_number - 1));
            }
        }
        if let Some(start) = range_start {
            missing_ranges.push((start, end_block));
        }

        for (missing_start, missing_end) in &missing_ranges {
            // Determine prev_digest
            let prev_digest = if let Some(prev_block) = self.blocks.get(&(missing_start - 1)) {
                H256::from(prev_block.digest.to_bytes_be()) // Assuming `digest` is the hash of the block
            } else {
                warn!(
                    "Cannot find previous block digest for missing range starting at {}",
                    missing_start
                );
                from_digest
            };

            debug!(
                "Fetching missing fragment from {} to {}, using prev_digest: {}",
                missing_start, missing_end, prev_digest
            );

            let fragment_manager = Manager::new(*missing_start, *missing_end, &self.eth_client);

            let fragment: CreateResult = crate::retry::ret(
                || async { fragment_manager.create(prev_digest).await },
                10,
                10,
                Some(60),
            )
            .await?;

            for block in fragment.continuity_proof.blocks() {
                self.blocks.insert(block.block_number, block.clone());
            }
        }

        // Construct final fragment only from the required range
        let final_fragment = AttestationFragment::from_blocks(
            (start_block..=end_block)
                .filter_map(|num| self.blocks.get(&num).cloned())
                .collect(),
        );

        for block in final_fragment.blocks() {
            debug!("Block({}) digest: {}", block.block_number, block.digest());
        }

        Ok(final_fragment)
    }
}
