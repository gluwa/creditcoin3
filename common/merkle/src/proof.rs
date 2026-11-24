//! Merkle proof implementation for transaction inclusion verification.
//!
//! This module provides a specialized Merkle proof structure designed for
//! transaction inclusion verification. It can be used in SDKs and precompiles.

use crate::keccak::{hash_inner, hash_leaf};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(feature = "precompile-support")]
use precompile_utils::prelude::String;
#[cfg(feature = "precompile-support")]
use precompile_utils::solidity::Codec;

/// Transaction inclusion Merkle proof structure
///
/// This structure maintains compatibility with the Solidity ABI while leveraging
/// Keccak256 hash functions. Used to prove that a transaction is included in a
/// block's Merkle tree.
#[derive(
    Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Default, Serialize, Deserialize,
)]
pub struct TransactionMerkleProof {
    /// The Merkle root hash
    pub root: H256,
    /// Sibling hashes with position information
    pub siblings: Vec<MerkleProofEntry>,
}

/// Maximum Merkle-proof depth accepted when ABI-decoding a proof in a precompile.
///
/// A proof of depth `d` proves inclusion in a tree of up to `2^d` leaves, so 64 is
/// already past any real block, and `calculate_tx_index_impl` rejects anything larger.
#[cfg(feature = "precompile-support")]
pub const MAX_MERKLE_SIBLINGS: u32 = 64;

/// Solidity `Codec` that limits `siblings` to [`MAX_MERKLE_SIBLINGS`] entries.
///
/// `write`, `signature` and `has_static_size` delegate to the plain field tuple, so the
/// ABI encoding and every precompile selector are the same as with `derive(Codec)`.
#[cfg(feature = "precompile-support")]
const _: () = {
    use precompile_utils::solidity::codec::{BoundedVec, Codec, Reader, String, Writer};
    use precompile_utils::solidity::revert::{BacktraceExt, MayRevert};
    use sp_core::ConstU32;

    impl Codec for TransactionMerkleProof {
        fn read(reader: &mut Reader) -> MayRevert<Self> {
            let (root, siblings): (
                H256,
                BoundedVec<MerkleProofEntry, ConstU32<MAX_MERKLE_SIBLINGS>>,
            ) = reader.read().map_in_tuple_to_field(&["root", "siblings"])?;
            Ok(Self {
                root,
                siblings: siblings.into(),
            })
        }

        fn write(writer: &mut Writer, value: Self) {
            Codec::write(writer, (value.root, value.siblings));
        }

        fn has_static_size() -> bool {
            <(H256, Vec<MerkleProofEntry>)>::has_static_size()
        }

        fn signature() -> String {
            <(H256, Vec<MerkleProofEntry>)>::signature()
        }
    }
};

/// A single entry in the merkle proof
#[derive(
    Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "precompile-support", derive(Codec))]
#[serde(rename_all = "camelCase")]
pub struct MerkleProofEntry {
    /// The sibling hash
    pub hash: H256,
    /// Indicates the relative position with respect to its sibling
    pub is_left: bool,
}

impl TransactionMerkleProof {
    /// Create a new TransactionMerkleProof
    pub fn new(root: H256, siblings: Vec<MerkleProofEntry>) -> Self {
        Self { root, siblings }
    }

    /// Verify the Merkle proof for transaction inclusion using Keccak256 hash
    pub fn verify(&self, tx_data: &[u8]) -> bool {
        let mut current_hash = hash_leaf(tx_data);

        // Traverse the Merkle path using siblings with position information
        for entry in &self.siblings {
            let (left, right) = if entry.is_left {
                (entry.hash.as_bytes(), current_hash.as_bytes())
            } else {
                (current_hash.as_bytes(), entry.hash.as_bytes())
            };

            current_hash = hash_inner(left, right);
        }

        // Compare computed root with provided root
        current_hash == self.root
    }

    /// Get the number of levels in the Merkle tree based on siblings
    pub fn levels(&self) -> usize {
        self.siblings.len()
    }

    /// Check if this is a single-transaction proof (no siblings)
    pub fn is_single_transaction(&self) -> bool {
        self.siblings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_proof_creation() {
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: false,
            },
        ];

        let proof = TransactionMerkleProof::new(root, siblings.clone());

        assert_eq!(proof.root, root);
        assert_eq!(proof.siblings, siblings);
        assert_eq!(proof.levels(), 2);
        assert!(!proof.is_single_transaction());
    }

    #[test]
    fn test_single_transaction_detection() {
        let root = H256::from([1u8; 32]);
        let proof = TransactionMerkleProof::new(root, vec![]);

        assert!(proof.is_single_transaction());
        assert_eq!(proof.levels(), 0);
    }
}
