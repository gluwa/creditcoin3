//! Shared utilities for continuity proof building
//!
//! This module contains common functions used by both indexer and CC3 chain
//! proof building paths.

use super::ContinuityBuilder;
use attestor_primitives::block::Block;
use sp_core::H256;

impl ContinuityBuilder {
    /// Helper: Add an attestation block to a chain of blocks.
    /// Computes the digest using the last block's digest as prev_digest, or falls back to provided prev_digest.
    pub(crate) fn add_attestation_block(
        &self,
        blocks: &mut Vec<Block>,
        block_number: u64,
        root: H256,
        fallback_prev_digest: Option<H256>,
    ) {
        let prev_digest = blocks
            .last()
            .map(|b| b.digest)
            .or(fallback_prev_digest)
            .unwrap_or_default();

        let digest = Block::hash_payload(&block_number, &root, &prev_digest);

        blocks.push(Block {
            block_number,
            root,
            prev_digest,
            digest,
        });
    }
}
