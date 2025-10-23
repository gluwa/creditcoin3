use anyhow::Result;
use sp_core::H256;
use std::collections::BTreeMap;
use thiserror::Error;
use tracing::{debug, error, info};

use eth::{
    continuity::{Error as ContinuityError, Manager},
    Client,
};

pub use attestor_primitives::{
    attestation_fragment::{AttestationFragment, AttestationFragmentSerializable},
    block::Block,
};
use ccnext_abi_encoding::abi::EncodingVersion;

#[derive(Debug, Clone)]
pub struct Cache {
    // Add fields here
    eth_client: Client,
    // Block cache
    blocks: BTreeMap<u64, Block>,
    // Block encoding
    encoding: EncodingVersion,
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
    pub fn new(eth_client: Client, encoding: EncodingVersion) -> Self {
        Cache {
            eth_client,
            blocks: BTreeMap::new(),
            encoding,
        }
    }

    /// Prunes cache by removing blocks older than the specified block number.
    pub fn prune_all_before(&mut self, block_number: u64) {
        // Remove all blocks before the specified block number
        debug!("Pruning cache. Removing blocks older than {}", block_number);
        self.blocks.retain(|&k, _| k >= block_number);
    }

    /// Creates a new attestation fragment from the specified block range.
    /// If any blocks in the range are missing from the local cache,
    /// they will be fetched using the `continuity_chain::Manager` and added to the cache.
    ///
    /// The function ensures digest continuity by:
    /// - Using the explicitly provided `from_digest` when the fragment starts at `start_block`
    /// - Falling back to the digest of the cached block at `missing_start - 1`
    /// - Using `from_digest` as a fallback if no previous block is cached
    ///
    /// # Arguments
    /// * `start_block` - The starting block number of the fragment (inclusive)
    /// * `from_digest` - The digest of the block immediately before `start_block`
    /// * `end_block` - The ending block number of the fragment (inclusive)
    ///
    /// # Returns
    /// * `AttestationFragment` if the range is successfully constructed
    ///
    /// # Errors
    /// * Returns `Error::InvalidParameters` if `start_block` > `end_block`
    /// * Returns `Error::ContinuityError` if fragment creation or retry fails
    pub async fn async_retry_create(
        &mut self,
        start_block: u64,
        from_digest: H256,
        end_block: u64,
    ) -> Result<AttestationFragment, Error> {
        // Validate input range
        if start_block > end_block {
            return Err(Error::InvalidParameters(start_block, end_block));
        }

        info!(
            "⛓️ Creating fragment from block {} to {}, provided from_digest: {:?}",
            start_block, end_block, from_digest
        );

        debug!("Cached blocks len: {}", self.blocks.len());

        // Determine block number ranges that are missing in the cache
        let mut missing_ranges = Vec::new(); // Vec to store (start, end) of missing block ranges
        let mut range_start = None; // Tracks start of a missing range

        for block_number in start_block..=end_block {
            if !self.blocks.contains_key(&block_number) {
                // Start of a missing range
                if range_start.is_none() {
                    range_start = Some(block_number);
                }
            } else if let Some(start) = range_start.take() {
                // End of a missing range
                missing_ranges.push((start, block_number - 1));
            }
        }

        // If we ended with a missing range at the tail end
        if let Some(start) = range_start {
            missing_ranges.push((start, end_block));
        }

        // Iterate through each missing range and fetch fragments
        for (missing_start, missing_end) in &missing_ranges {
            // Determine the digest to use for the fragment's starting point
            let prev_digest = if *missing_start == start_block {
                // If this is the first range, use the explicitly provided digest
                debug!(
                    "❔Missing fragment starts at {}, using provided from_digest: {:?}",
                    missing_start, from_digest
                );
                from_digest
            } else if let Some(prev_block) = self.blocks.get(&(missing_start - 1)) {
                // Use the digest of the block immediately before the missing range
                debug!(
                    "❔Missing fragment starts at {}, using previous block digest: {:?}",
                    missing_start,
                    prev_block.digest()
                );
                H256::from(prev_block.digest().to_bytes_be())
            } else {
                // Fallback: no digest available for previous block, use the passed one
                debug!(
                    "❔Cannot find previous block digest for missing range starting at {:?}",
                    missing_start
                );
                from_digest
            };

            // Log fragment fetch details
            debug!(
                "Fetching missing fragment from {} to {}, using prev_digest: {}",
                missing_start, missing_end, prev_digest
            );

            // Create a new fragment manager for the given range
            let fragment_manager = Manager::new(*missing_start, *missing_end, &self.eth_client);

            // Use retry logic to ensure fragment creation is attempted multiple times
            let fragment: AttestationFragment = crate::util::retry::ret(
                || async { fragment_manager.create(prev_digest, self.encoding).await },
                10,       // max_retries
                10,       // delay between retries (seconds)
                Some(60), // max retry duration (seconds)
            )
            .await?;

            // Insert the newly fetched blocks into the local cache
            for block in fragment.blocks() {
                self.blocks.insert(block.block_number, block.clone());
            }
        }

        // Construct the final fragment from the now-complete block range in cache
        let final_fragment = AttestationFragment::from_blocks(
            (start_block..=end_block)
                .filter_map(|num| self.blocks.get(&num).cloned())
                .collect(),
        );

        // Log the final fragment's blocks and their digests
        for block in final_fragment.blocks() {
            debug!(
                "Block({}) digest: {:?}, root: {:?} prev_digest: {:?}",
                block.block_number,
                H256::from(block.digest().to_bytes_be()),
                H256::from(block.root.to_bytes_be()),
                H256::from(block.prev_digest().to_bytes_be())
            );
        }

        Ok(final_fragment)
    }
}
