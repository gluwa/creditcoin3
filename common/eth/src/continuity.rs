use anyhow::Result;
use futures::stream::{self, StreamExt};
use sp_core::H256;
use tracing::{debug, trace};
use usc_abi_encoding::common::EncodingVersion;
use user::prelude::*;

use super::{Client, Error as EthError};
use attestor_primitives::block::{Block, BlockError};

/// Maximum number of concurrent block fetches when building continuity chains.
/// This limits concurrency to avoid overwhelming Redis with too many simultaneous requests.
/// Redis supports concurrent connections via multiplexed connections, but limiting concurrency
/// prevents timeouts and ensures better performance when fetching many blocks.
const MAX_CONCURRENT_BLOCK_FETCHES: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid fragment length, {0}")]
    InvalidFragmentLength(u64),
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
    ) -> Result<Vec<Block>, Error> {
        debug!(
            "Building continuity blocks for interval: {} - {}",
            self.start_block, self.end_block
        );

        // Get all blocks with limited concurrency to avoid overwhelming Redis
        let blocks = stream::iter(self.start_block..=self.end_block)
            .map(|i| self.eth_client.get_block(i, encoding))
            .buffered(MAX_CONCURRENT_BLOCK_FETCHES)
            .collect::<Vec<_>>()
            .await;

        let collected_blocks = blocks
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap_interrupt("Not handling user interrupts yet")?;

        // Spawn MMR computations in parallel threads
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

        // Build chain of blocks with digest continuity
        let mut result: Vec<Block> = Vec::with_capacity(blocks_with_roots.len());
        let mut prev_digest = prev_digest;

        for block_with_root in blocks_with_roots {
            let (block, root) = block_with_root?;
            let next_block = Block::new_from_prev_digest(block.number(), root, prev_digest);

            let block = if let Some(prev) = result.last() {
                if next_block.block_number != prev.block_number + 1 {
                    return Err(BlockError::BlockNumberMismatch(next_block.block_number).into());
                }
                next_block
            } else {
                debug!("Constructing first block from start block");
                next_block
            };

            trace!(
                "Appending block number: {} with root: {:?}",
                block.block_number,
                block.root
            );
            prev_digest = block.digest;
            result.push(block);
        }

        Ok(result)
    }
}
