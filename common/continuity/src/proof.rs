//! Continuity proof types and construction.
//!
//! This module defines the structure of continuity proofs and provides
//! constructors for building them from block data.

use attestor_primitives::block::{Block, ContinuityProof as AttestorContinuityProof};
use serde::Deserialize;
use sp_core::H256;

/// A precomputed continuity proof linking source chain blocks to attestations.
///
/// This is a built continuity proof that sources data from the indexer. It contains
/// full block structures with all metadata needed for proof construction and verification.
///
/// This proof allows on-chain verification that a sequence of source chain blocks
/// was correctly attested to by the Creditcoin3 network. The proof consists of:
///
/// 1. A chain of blocks starting from `queryHeight - 1`
/// 2. The query block(s) being proven
/// 3. Additional blocks up to the next attestation
///
/// # Structure
///
/// The proof is valid if:
/// - Each block's digest correctly links to the next (via prev_digest)
/// - The chain starts at the lower endpoint
/// - The chain ends at an attestation consensus point
///
/// # Examples
///
/// ```rust
/// use continuity::BuiltContinuityProof;
/// use attestor_primitives::block::Block;
/// use sp_core::H256;
///
/// let blocks = vec![
///     Block {
///         block_number: 99,
///         root: H256::random(),
///         prev_digest: H256::zero(),
///         digest: H256::random(),
///     },
///     Block {
///         block_number: 100,
///         root: H256::random(),
///         prev_digest: H256::random(),
///         digest: H256::random(),
///     },
/// ];
///
/// let proof = BuiltContinuityProof::from_blocks(blocks);
/// assert_eq!(proof.len(), 2);
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct BuiltContinuityProof {
    /// The sequence of blocks forming the continuity chain.
    ///
    /// Ordered by block number ascending. The first block is at `queryHeight - 1`
    /// and the last block is at the next attestation after the query.
    pub blocks: Vec<Block>,

    /// Lower endpoint digest (digest of the lower attestation block).
    ///
    /// This is set when blocks are trimmed and the first block's prev_digest
    /// doesn't correspond to the lower attestation digest. It allows the verifier
    /// to link the proof to the correct attestation boundary.
    pub lower_endpoint_digest: Option<H256>,
}

impl BuiltContinuityProof {
    /// Create a proof from a sequence of blocks.
    ///
    /// # Arguments
    ///
    /// * `blocks` - Ordered sequence of blocks forming the continuity chain
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            lower_endpoint_digest: None,
        }
    }

    /// Create a proof with an explicit lower endpoint digest.
    ///
    /// Used when the continuity chain has been trimmed and doesn't start at
    /// the lower attestation block itself.
    ///
    /// # Arguments
    ///
    /// * `blocks` - Ordered sequence of blocks
    /// * `lower_endpoint_digest` - Digest of the lower attestation
    pub fn from_blocks_with_lower_digest(blocks: Vec<Block>, lower_endpoint_digest: H256) -> Self {
        Self {
            blocks,
            lower_endpoint_digest: Some(lower_endpoint_digest),
        }
    }

    /// Get the number of blocks in this proof.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Check if the proof contains no blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Convert to the on-chain optimized ContinuityProof format.
    ///
    /// This extracts only the roots (digests are computed on-chain) and resolves
    /// the lower_endpoint_digest. If `lower_endpoint_digest` is set, it uses that.
    /// Otherwise, it uses the first block's `prev_digest`.
    ///
    /// # Returns
    ///
    /// Returns `None` if the proof is empty and no lower_endpoint_digest is set.
    pub fn to_attestor_proof(&self) -> Option<AttestorContinuityProof> {
        if self.blocks.is_empty() {
            return self
                .lower_endpoint_digest
                .map(|digest| AttestorContinuityProof {
                    lower_endpoint_digest: digest,
                    roots: Vec::new(),
                });
        }

        let lower_endpoint_digest = self
            .lower_endpoint_digest
            .unwrap_or_else(|| self.blocks[0].prev_digest);

        let roots: Vec<H256> = self.blocks.iter().map(|b| b.root).collect();

        Some(AttestorContinuityProof {
            lower_endpoint_digest,
            roots,
        })
    }

    /// Convert to the on-chain optimized ContinuityProof format with smart lower digest resolution.
    ///
    /// This method uses the stored `lower_endpoint_digest` if available, otherwise falls back
    /// to the provided `lower_attestation_digest`.
    ///
    /// # Arguments
    ///
    /// * `lower_attestation_digest` - The digest of the lower attestation (fallback)
    ///
    /// # Returns
    ///
    /// Returns `None` if the proof is empty and no lower_endpoint_digest can be determined.
    pub fn to_attestor_proof_with_attestation_context(
        &self,
        lower_attestation_digest: H256,
    ) -> Option<AttestorContinuityProof> {
        if self.blocks.is_empty() {
            return Some(AttestorContinuityProof {
                lower_endpoint_digest: self
                    .lower_endpoint_digest
                    .unwrap_or(lower_attestation_digest),
                roots: Vec::new(),
            });
        }

        let lower_endpoint_digest = self
            .lower_endpoint_digest
            .unwrap_or(lower_attestation_digest);

        let roots: Vec<H256> = self.blocks.iter().map(|b| b.root).collect();

        Some(AttestorContinuityProof {
            lower_endpoint_digest,
            roots,
        })
    }
}

impl From<BuiltContinuityProof> for AttestorContinuityProof {
    fn from(proof: BuiltContinuityProof) -> Self {
        proof.to_attestor_proof().unwrap_or_default()
    }
}

impl From<&BuiltContinuityProof> for AttestorContinuityProof {
    fn from(proof: &BuiltContinuityProof) -> Self {
        proof.to_attestor_proof().unwrap_or_default()
    }
}
