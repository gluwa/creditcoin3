use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use precompile_utils::solidity::Codec;
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(feature = "std")]
use std::string::String;

#[cfg(not(feature = "std"))]
use sp_runtime::format;

// Removed Felt import - using H256 instead

pub trait MaybeCreatedFromEmpty {
    fn created_from_empty(&self) -> bool;
}

#[derive(Debug)]
pub enum BlockError {
    BlockNumberMismatch(u64),
    Empty(u64),
}

#[cfg(feature = "std")]
impl core::fmt::Display for BlockError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BlockError::BlockNumberMismatch(n) => {
                write!(f, "Block number mismatch: expected {}, got {}", n - 1, n)
            }
            BlockError::Empty(n) => write!(f, "Block {n} is empty"),
        }
    }
}

#[cfg(feature = "std")]
impl core::error::Error for BlockError {}

#[derive(Debug, Clone, Default, Codec, Deserialize)]
pub struct Block {
    pub block_number: u64,
    pub root: H256,
    pub prev_digest: H256,
    pub digest: H256,
}

impl Block {
    pub fn new(block_number: u64, root: H256) -> Self {
        let prev_digest = H256::default();
        let digest = Self::hash_payload(&block_number, &root, &prev_digest);

        Self {
            block_number,
            root,
            prev_digest,
            digest,
        }
    }

    /// Creates a new block with the given block number, root, and previous digest.
    pub fn new_from_prev_digest(block_number: u64, root: H256, prev_digest: H256) -> Self {
        let digest = Self::hash_payload(&block_number, &root, &prev_digest);

        Self {
            block_number,
            root,
            prev_digest,
            digest,
        }
    }

    /// Creates a new block with the given block number and digest, initializing root and prev_digest to default values.
    pub fn new_from_digest(block_number: u64, _root: H256, digest: H256) -> Self {
        Self {
            block_number,
            root: Default::default(),
            prev_digest: Default::default(),
            digest,
        }
    }

    pub fn n(&self) -> u64 {
        self.block_number
    }

    pub fn digest(&self) -> H256 {
        self.digest
    }

    pub fn prev_digest(&self) -> H256 {
        self.prev_digest
    }

    pub fn try_from_previous(prev: &Self, block: Self) -> Result<Self, BlockError> {
        if block.block_number != prev.block_number + 1 {
            return Err(BlockError::BlockNumberMismatch(block.block_number));
        }
        let digest = Self::hash_payload(&block.block_number, &block.root, &prev.digest);

        Ok(Self {
            block_number: block.block_number,
            root: block.root,
            prev_digest: prev.digest,
            digest,
        })
    }

    pub fn from_block_number_and_digest(block_number: u64, digest: H256) -> Self {
        Self {
            block_number,
            digest,
            ..Default::default()
        }
    }

    pub fn hash_payload(block_number: &u64, root: &H256, prev_digest: &H256) -> H256 {
        use sp_io::hashing::keccak_256;
        // Pre-allocate: 8 bytes (u64) + 32 bytes (H256) + 32 bytes (H256) = 72 bytes
        let mut bytes = Vec::with_capacity(8 + 32 + 32);
        bytes.extend_from_slice(&block_number.to_be_bytes());
        bytes.extend_from_slice(root.as_bytes());
        bytes.extend_from_slice(prev_digest.as_bytes());
        H256::from(keccak_256(&bytes))
    }
}

impl MaybeCreatedFromEmpty for Block {
    fn created_from_empty(&self) -> bool {
        self.root == Default::default()
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Encode,
    Decode,
    MaxEncodedLen,
    TypeInfo,
    Default,
    Serialize,
    Deserialize,
)]
pub struct BlockSerializable {
    pub block_number: u64,
    pub root: H256,
    pub prev_digest: H256,
    pub digest: H256,
}

/// Optimized continuity block structure for native query verifier
/// Contains only root and digest (block_number and prev_digest are inferred from continuity proof structure)
#[derive(Debug, Clone, Default, Codec, Serialize, Deserialize)]
pub struct ContinuityBlock {
    pub merkle_root: H256,
    pub digest: H256,
}

#[derive(
    Debug, Clone, TypeInfo, Decode, Encode, PartialEq, Eq, Default, Serialize, Deserialize,
)]
pub struct ContinuityBlockSerializable {
    merkle_root: H256,
    digest: H256,
}

impl From<&ContinuityBlock> for ContinuityBlockSerializable {
    fn from(b: &ContinuityBlock) -> Self {
        Self {
            merkle_root: b.merkle_root,
            digest: b.digest,
        }
    }
}

impl From<BlockSerializable> for ContinuityBlockSerializable {
    fn from(b: BlockSerializable) -> Self {
        Self {
            merkle_root: b.root,
            digest: b.digest,
        }
    }
}

impl TryFrom<ContinuityBlockSerializable> for ContinuityBlock {
    type Error = ();

    fn try_from(block: ContinuityBlockSerializable) -> Result<Self, Self::Error> {
        Ok(Self {
            merkle_root: block.merkle_root,
            digest: block.digest,
        })
    }
}

/// Optimized continuity proof structure for native query verifier
///
/// Reduces calldata size by:
/// - Block numbers are inferred from query height(s) and index
///   - Single query: blocks[0] is at queryHeight - 1
///   - Batch queries: blocks[0] is at min(queryHeights) - 1
/// - prev_digest is reconstructed from the chain (using lower_endpoint_digest and computed digests)
/// - Keeping only root and digest per block
#[derive(Debug, Clone, Default, Codec, Serialize, Deserialize)]
pub struct ContinuityProof {
    /// The digest of the block before the continuity chain starts
    /// This is the prev_digest of the first block
    pub lower_endpoint_digest: H256,
    /// Array of continuity blocks (each containing only root and digest)
    /// Block numbers are inferred: blocks[i] is at (queryHeight - 1) + i for single query
    pub blocks: Vec<ContinuityBlock>,
}

impl ContinuityProof {
    /// Create a new continuity proof from a lower endpoint digest and blocks
    pub fn new(lower_endpoint_digest: H256, blocks: Vec<ContinuityBlock>) -> Self {
        Self {
            lower_endpoint_digest,
            blocks,
        }
    }

    /// Convert from Vec<Block> to ContinuityProof
    /// Extracts the prev_digest from the first block
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            return Self::default();
        }

        // The lower_endpoint_digest is the prev_digest of the first block
        let lower_endpoint_digest = blocks[0].prev_digest;

        // Convert blocks to ContinuityBlocks (dropping block_number and prev_digest)
        // prev_digest will be reconstructed from the chain when converting back
        let continuity_blocks: Vec<ContinuityBlock> = blocks
            .into_iter()
            .map(|b| ContinuityBlock {
                merkle_root: b.root,
                digest: b.digest,
            })
            .collect();

        Self {
            lower_endpoint_digest,
            blocks: continuity_blocks,
        }
    }

    /// Convert ContinuityProof back to Vec<Block> given the starting block number
    /// Reconstructs prev_digest from the chain using lower_endpoint_digest and computed digests
    pub fn to_blocks(&self, start_block_number: u64) -> Vec<Block> {
        let mut blocks = Vec::with_capacity(self.blocks.len());
        let mut prev_digest = self.lower_endpoint_digest;

        for (idx, cb) in self.blocks.iter().enumerate() {
            let block_number = start_block_number + idx as u64;
            // Reconstruct prev_digest from the chain
            // Start with lower_endpoint_digest, then use each block's computed digest
            let block = Block {
                block_number,
                root: cb.merkle_root,
                prev_digest,
                digest: cb.digest,
            };
            // Use the stored digest as the next block's prev_digest
            prev_digest = cb.digest;
            blocks.push(block);
        }

        blocks
    }
}

impl From<&Block> for BlockSerializable {
    fn from(b: &Block) -> Self {
        Self {
            block_number: b.block_number,
            root: b.root,
            prev_digest: b.prev_digest,
            digest: b.digest,
        }
    }
}

impl From<BlockSerializable> for Block {
    fn from(val: BlockSerializable) -> Self {
        Block {
            block_number: val.block_number,
            root: val.root,
            prev_digest: val.prev_digest,
            digest: val.digest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h256_from_u64(n: u64) -> H256 {
        let mut bytes = [0u8; 32];
        bytes[24..32].copy_from_slice(&n.to_be_bytes());
        H256::from(bytes)
    }

    #[test]
    fn continuity_block_serialization_is_hex_and_roundtrips() {
        // Arrange
        let original = ContinuityBlock {
            merkle_root: h256_from_u64(123456789012345678),
            digest: h256_from_u64(42),
        };

        // Act: produce serializable and serialize to JSON
        let ser = ContinuityBlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");

        // Assert: serialized strings are hex (with 0x prefix)
        assert!(
            json.contains("0x"),
            "Serialized ContinuityBlockSerializable fields should be hex strings with 0x prefix"
        );

        // Act: parse back to ContinuityBlockSerializable via JSON and then convert
        let parsed_ser: ContinuityBlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = ContinuityBlock::try_from(parsed_ser)
            .expect("parsing ContinuityBlockSerializable failed");

        // Assert: values round-trip
        assert_eq!(original.merkle_root, parsed.merkle_root);
        assert_eq!(original.digest, parsed.digest);
    }

    #[test]
    fn block_serialization_is_hex_and_roundtrips() {
        // Arrange: a Block with H256 values
        let original = Block {
            block_number: 7,
            root: h256_from_u64(98765432109876543),
            prev_digest: h256_from_u64(314159265358979),
            digest: h256_from_u64(271828182845904),
        };

        // Act: produce serializable and serialize to JSON
        let ser = BlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");

        // Assert: serialized strings are hex (with 0x prefix)
        assert!(json.contains("0x"));

        // Act: parse back via JSON and convert to Block
        // Act: parse back to BlockSerializable via JSON and then convert
        let parsed_ser: BlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = Block::from(parsed_ser);

        // Assert: values round-trip
        assert_eq!(original.block_number, parsed.block_number);
        assert_eq!(original.root, parsed.root);
        assert_eq!(original.prev_digest, parsed.prev_digest);
        assert_eq!(original.digest, parsed.digest);
    }

    #[test]
    fn precise_h256_serialization_format() {
        // Use distinct, simple numeric seeds to make expected hex easy to reason about.
        let block = Block {
            block_number: 42,
            root: h256_from_u64(1),
            prev_digest: h256_from_u64(2),
            digest: h256_from_u64(3),
        };
        let ser_block = BlockSerializable::from(&block);
        let value = serde_json::to_value(&ser_block).expect("serialize block");

        // Helper to assert exact 0x-prefixed lowercase 64 hex chars.
        let assert_h256_str = |label: &str, s: &str| {
            assert!(s.starts_with("0x"), "{label} must start with 0x: {s}");
            assert_eq!(
                s.len(),
                66,
                "{label} must be 0x + 64 hex chars (len=66), got len={}",
                s.len()
            );
            assert!(
                s.chars().skip(2).all(
                    |c| c.is_ascii_hexdigit() && (c.is_ascii_lowercase() || c.is_ascii_digit())
                ),
                "{label} must be lowercase hex (0-9a-f). Got: {s}"
            );
        };

        // Extract strings
        let root_str = value
            .get("root")
            .and_then(|v| v.as_str())
            .expect("root string");
        let prev_str = value
            .get("prev_digest")
            .and_then(|v| v.as_str())
            .expect("prev_digest string");
        let digest_str = value
            .get("digest")
            .and_then(|v| v.as_str())
            .expect("digest string");

        assert_h256_str("root", root_str);
        assert_h256_str("prev_digest", prev_str);
        assert_h256_str("digest", digest_str);

        // Continuity block serialization check as well.
        let cblock = ContinuityBlock {
            merkle_root: block.root,
            digest: block.digest,
        };
        let ser_cblock = ContinuityBlockSerializable::from(&cblock);
        let cval = serde_json::to_value(&ser_cblock).expect("serialize continuity block");
        let c_root = cval
            .get("root")
            .and_then(|v| v.as_str())
            .expect("continuity root");
        let c_digest = cval
            .get("digest")
            .and_then(|v| v.as_str())
            .expect("continuity digest");
        assert_h256_str("continuity.root", c_root);
        assert_h256_str("continuity.digest", c_digest);
    }
}
