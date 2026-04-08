use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use precompile_utils::solidity::Codec;
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(feature = "std")]
use std::string::String;

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
        crate::compute_digest_for(*block_number, root, Some(prev_digest))
    }
}

impl MaybeCreatedFromEmpty for Block {
    fn created_from_empty(&self) -> bool {
        self.root == Default::default()
    }
}

/// Serialization format for continuity proof blocks.
/// Digest and prev_digest are omitted - they are computed from the chain.
/// prev_digest for block 0 comes from the proof's lower_endpoint_digest;
/// for block i > 0 it is the digest of block i-1.
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

impl BlockSerializable {
    /// Create ContinuityBlockSerializable; caller must provide prev_digest for the chain.
    pub fn to_continuity_block(&self, prev_digest: H256) -> ContinuityBlockSerializable {
        let digest = Block::hash_payload(&self.block_number, &self.root, &prev_digest);
        ContinuityBlockSerializable {
            merkle_root: self.root,
            digest,
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

/// Simplified continuity proof structure matching BlockProver.sol
/// Only stores roots - digests are computed on-chain
///
/// ABI structure: (bytes32, bytes32[])
/// Tuple with lowerEndpointDigest and array of roots
/// Block number for index i = startBlock + i, where startBlock = queryBlockHeight
/// The query block is at index 0 for optimal proof size
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Codec, Serialize, Deserialize, Encode, Decode, TypeInfo,
)]
#[serde(rename_all = "camelCase")]
pub struct ContinuityProof {
    /// The digest of the block before the continuity chain starts (digest of queryHeight - 1)
    pub lower_endpoint_digest: H256,
    /// Array of merkle roots (digests computed on-chain)
    /// Block number for index i = startBlock + i, where startBlock = queryBlockHeight
    pub roots: Vec<H256>,
}

impl ContinuityProof {
    /// Create a new simplified continuity proof from a lower endpoint digest and roots
    pub fn new(lower_endpoint_digest: H256, roots: Vec<H256>) -> Self {
        Self {
            lower_endpoint_digest,
            roots,
        }
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Get digest for block at index i (0-based). Returns the digest of the block at start_block_number + i.
    pub fn digest_at(&self, start_block_number: u64, index: usize) -> Option<H256> {
        if index >= self.roots.len() {
            return None;
        }
        let mut prev = self.lower_endpoint_digest;
        for i in 0..=index {
            prev = Self::hash_payload(&(start_block_number + i as u64), &self.roots[i], &prev);
        }
        Some(prev)
    }

    /// Compute digest for block at given height. Returns None if height is outside the proof range.
    pub fn digest_for_block(&self, start_block_number: u64, block_number: u64) -> Option<H256> {
        if block_number < start_block_number {
            return None;
        }
        let index = (block_number - start_block_number) as usize;
        self.digest_at(start_block_number, index)
    }

    /// Convert from Vec<Block> to ContinuityProof
    /// Extracts the prev_digest from the first block and collects only roots
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            return Self::default();
        }

        // The lower_endpoint_digest is the prev_digest of the first block
        let lower_endpoint_digest = blocks[0].prev_digest;

        // Extract only roots (digests will be computed on-chain)
        let roots: Vec<H256> = blocks.into_iter().map(|b| b.root).collect();

        Self {
            lower_endpoint_digest,
            roots,
        }
    }

    /// Convert ContinuityProof back to Vec<Block> given the starting block number
    /// Computes digests on-chain using hash_payload
    pub fn to_blocks(&self, start_block_number: u64) -> Vec<Block> {
        let mut blocks = Vec::with_capacity(self.roots.len());
        let mut prev_digest = self.lower_endpoint_digest;

        for (idx, root) in self.roots.iter().enumerate() {
            let block_number = start_block_number + idx as u64;
            // Compute digest on-chain
            let digest = Self::hash_payload(&block_number, root, &prev_digest);

            let block = Block {
                block_number,
                root: *root,
                prev_digest,
                digest,
            };
            // Use the computed digest as the next block's prev_digest
            prev_digest = digest;
            blocks.push(block);
        }

        blocks
    }

    /// Compute block digest: keccak256(blockNumber || merkleRoot || prevDigest)
    /// Matches BlockProver.sol computeBlockDigest function
    /// Single hash of 72 bytes: 8 bytes (uint64) + 32 bytes (bytes32) + 32 bytes (bytes32)
    pub fn hash_payload(block_number: &u64, merkle_root: &H256, prev_digest: &H256) -> H256 {
        Block::hash_payload(block_number, merkle_root, prev_digest)
    }

    /// Compute continuity digest chain
    /// Matches BlockProver.sol computeContinuityDigest function
    pub fn compute_continuity_digest(&self, start_block: u64) -> H256 {
        let mut digest = self.lower_endpoint_digest;

        for (i, root) in self.roots.iter().enumerate() {
            // Compute block number from start + index
            let block_number = start_block + i as u64;
            // Compute next digest
            digest = Self::hash_payload(&block_number, root, &digest);
        }

        digest
    }

    /// First block number in the proof given the attestation header (last block).
    /// The proof contains blocks [start_block_number, attestation_header_number - 1].
    pub fn start_block_number(&self, attestation_header_number: u64) -> u64 {
        attestation_header_number.saturating_sub(self.roots.len() as u64)
    }

    /// Digest that the continuity proof chains from (prev_digest of the first block).
    /// Used for continuity validation to look up the finalized attestation/checkpoint.
    pub fn tail_prev_digest(&self) -> Option<H256> {
        if self.roots.is_empty() {
            return None;
        }
        Some(self.lower_endpoint_digest)
    }

    /// Build block_number -> digest map for checkpoint creation.
    pub fn block_digests(&self, start_block_number: u64) -> BTreeMap<u64, H256> {
        let mut map = BTreeMap::new();
        let mut prev = self.lower_endpoint_digest;
        for (i, root) in self.roots.iter().enumerate() {
            let block_number = start_block_number + i as u64;
            let digest = Self::hash_payload(&block_number, root, &prev);
            map.insert(block_number, digest);
            prev = digest;
        }
        map
    }

    /// Find query block index in continuity proof
    /// Returns the index of the block with the given height, or None if not found
    pub fn find_query_block_index(
        &self,
        start_block_number: u64,
        query_height: u64,
    ) -> Option<usize> {
        if self.roots.is_empty() {
            return None;
        }

        let first_block_num = start_block_number;
        let last_block_num = start_block_number + (self.roots.len() - 1) as u64;

        if query_height >= first_block_num && query_height <= last_block_num {
            let index = (query_height - first_block_num) as usize;
            if index < self.roots.len() {
                return Some(index);
            }
        }

        None
    }
}

impl From<&Block> for BlockSerializable {
    fn from(b: &Block) -> Self {
        Self {
            block_number: b.block_number,
            root: b.root,
        }
    }
}

impl From<Block> for BlockSerializable {
    fn from(b: Block) -> Self {
        Self {
            block_number: b.block_number,
            root: b.root,
        }
    }
}

impl Block {
    /// Create Block from BlockSerializable given the previous block's digest in the chain.
    pub fn from_serializable_with_prev(val: &BlockSerializable, prev_digest: H256) -> Self {
        let digest = Self::hash_payload(&val.block_number, &val.root, &prev_digest);
        Block {
            block_number: val.block_number,
            root: val.root,
            prev_digest,
            digest,
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
            digest: Block::hash_payload(
                &7,
                &h256_from_u64(98765432109876543),
                &h256_from_u64(314159265358979),
            ),
        };

        // Act: produce serializable (digest omitted) and serialize to JSON
        let ser = BlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");

        // Assert: serialized strings are hex (with 0x prefix)
        assert!(json.contains("0x"));

        // Act: parse back via JSON and convert to Block (digest is computed)
        let parsed_ser: BlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = Block::from_serializable_with_prev(&parsed_ser, original.prev_digest);

        // Assert: values round-trip (digest is recomputed)
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
            digest: Block::hash_payload(&42, &h256_from_u64(1), &h256_from_u64(2)),
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

        // BlockSerializable omits digest and prev_digest - only block_number and root are serialized
        let root_str = value
            .get("root")
            .and_then(|v| v.as_str())
            .expect("root string");

        assert_h256_str("root", root_str);

        // Continuity block serialization check as well.
        let cblock = ContinuityBlock {
            merkle_root: block.root,
            digest: block.digest,
        };
        let ser_cblock = ContinuityBlockSerializable::from(&cblock);
        let cval = serde_json::to_value(&ser_cblock).expect("serialize continuity block");
        let c_root = cval
            .get("merkle_root")
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
