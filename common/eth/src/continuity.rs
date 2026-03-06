use anyhow::Result;
use futures::stream::{self, StreamExt};
use sp_core::H256;
use tracing::{debug, trace};
use usc_abi_encoding::common::EncodingVersion;

use super::{Client, Error as EthError};
use attestor_primitives::{
    attestation_fragment::{AttestationFragment, AttestationFragmentError},
    block::{Block as FragmentBlock, BlockError},
};
use user::prelude::*;

/// Maximum number of concurrent block fetches when building continuity chains.
/// This limits concurrency to avoid overwhelming Redis with too many simultaneous requests.
/// Redis supports concurrent connections via multiplexed connections, but limiting concurrency
/// prevents timeouts and ensures better performance when fetching many blocks.
const MAX_CONCURRENT_BLOCK_FETCHES: usize = 20;

// Removed Felt import - using H256 instead

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid Fragment Length, {0}")]
    InvalidFragmentLength(u64),
    #[error("Attestation fragment error: {0}")]
    Fragment(#[from] AttestationFragmentError),
    #[error("Attestation fragment block eth error: {0}")]
    Eth(#[from] EthError),
    #[error("Attestation fragment block error: {0}")]
    BlockError(#[from] BlockError),
    #[error("MMR computation join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}

pub struct Manager<'a> {
    start_block: u64,
    end_block: u64,
    eth_client: &'a Client,
}

impl<'a> Manager<'a> {
    pub fn new(start_block: u64, end_block: u64, eth_client: &'a Client) -> Self {
        Self {
            start_block,
            end_block,
            eth_client,
        }
    }

    pub async fn create(
        &self,
        prev_digest: H256,
        encoding: EncodingVersion,
    ) -> Result<AttestationFragment, Error> {
        // Only for genesis block we don't need to build a fragment
        // if self.end_block == 0 {
        //     debug!("No need to build full fragment for genesis block");
        //     return Ok(AttestationFragment::default());
        // }

        // Fragment size is the difference between the attestation header number and the last finalized attestation header number
        // Start and end block are inclusive
        let fragment_size = self.end_block - self.start_block + 1;
        let fragment_length = usize::try_from(fragment_size)
            .map_err(|_| Error::InvalidFragmentLength(fragment_size))?;

        // Create a new fragment with the correct length
        let mut fragment = AttestationFragment::new(fragment_length);

        debug!(
            "Building fragment for interval: {} - {}",
            self.start_block, self.end_block
        );

        // Get all blocks with limited concurrency to avoid overwhelming Redis
        // This list is sorted because we provide the futures in order
        let blocks = stream::iter(self.start_block..=self.end_block)
            .map(|i| self.eth_client.get_block(i, encoding))
            .buffered(MAX_CONCURRENT_BLOCK_FETCHES)
            .collect::<Vec<_>>()
            .await;

        // Handle errors and collect blocks
        let collected_blocks = blocks
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap_interrupt("Not handling user interrupts")?;

        // Now spawn MMR computations in parallel threads
        let blocks_with_roots = stream::iter(collected_blocks)
            .map(|block| {
                let end_block = self.end_block;
                tokio::task::spawn_blocking(move || {
                    trace!("Merkleization of block {}/{}", block.number(), end_block);
                    let tree = crate::simple_merkle_tree(&block);
                    let root = tree.root();
                    (block, root)
                })
            })
            .buffered(10)
            .collect::<Vec<_>>()
            .await;

        // Start building the fragment for the interval
        for block_with_root in blocks_with_roots {
            let (block, root) = block_with_root?;

            let fragment_block = if fragment.is_empty() {
                debug!("Constructing first block from start block");
                FragmentBlock::new_from_prev_digest(block.number(), root, prev_digest)
            } else {
                FragmentBlock::new(block.number(), root)
            };

            trace!(
                "Appending block number: {} with root: {:?}",
                fragment_block.block_number,
                fragment_block.root
            );
            fragment.try_append_block(fragment_block)?;
        }

        Ok(fragment)
    }
}
