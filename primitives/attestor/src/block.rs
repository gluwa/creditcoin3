use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{de::Deserializer, Deserialize, Serialize, Serializer};
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
#[cfg(feature = "std")]
use std::format;

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

#[derive(Debug, Clone, Default)]
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
        let mut bytes = Vec::new();
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

// Helper serde adapters to encode/decode H256 as hex strings in JSON
fn h256_serialize_as_hex<S>(val: &H256, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&format!("0x{}", hex::encode(val.as_bytes())))
}

fn h256_deserialize_from_hex<'de, D>(d: D) -> Result<H256, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    let hex_str = s.strip_prefix("0x").unwrap_or(&s);
    let bytes = hex::decode(hex_str)
        .map_err(|e| serde::de::Error::custom(format!("Hex decode error: {e}")))?;
    if bytes.len() != 32 {
        return Err(serde::de::Error::custom(format!(
            "Expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(H256::from(arr))
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
    #[serde(
        serialize_with = "h256_serialize_as_hex",
        deserialize_with = "h256_deserialize_from_hex"
    )]
    pub root: H256,
    #[serde(
        serialize_with = "h256_serialize_as_hex",
        deserialize_with = "h256_deserialize_from_hex"
    )]
    pub prev_digest: H256,
    #[serde(
        serialize_with = "h256_serialize_as_hex",
        deserialize_with = "h256_deserialize_from_hex"
    )]
    pub digest: H256,
}

#[derive(Debug, Clone, Default)]
pub struct ContinuityBlock {
    root: H256,
    digest: H256,
}

#[derive(
    Debug, Clone, TypeInfo, Decode, Encode, PartialEq, Eq, Default, Serialize, Deserialize,
)]
pub struct ContinuityBlockSerializable {
    #[serde(
        serialize_with = "h256_serialize_as_hex",
        deserialize_with = "h256_deserialize_from_hex"
    )]
    root: H256,
    #[serde(
        serialize_with = "h256_serialize_as_hex",
        deserialize_with = "h256_deserialize_from_hex"
    )]
    digest: H256,
}

impl From<&ContinuityBlock> for ContinuityBlockSerializable {
    fn from(b: &ContinuityBlock) -> Self {
        Self {
            root: b.root,
            digest: b.digest,
        }
    }
}

impl From<BlockSerializable> for ContinuityBlockSerializable {
    fn from(b: BlockSerializable) -> Self {
        Self {
            root: b.root,
            digest: b.digest,
        }
    }
}

impl TryFrom<ContinuityBlockSerializable> for ContinuityBlock {
    type Error = ();

    fn try_from(block: ContinuityBlockSerializable) -> Result<Self, Self::Error> {
        Ok(Self {
            root: block.root,
            digest: block.digest,
        })
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
            root: h256_from_u64(123456789012345678),
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
        assert_eq!(original.root, parsed.root);
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
}
