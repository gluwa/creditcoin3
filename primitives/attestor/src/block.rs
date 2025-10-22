use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{de::Deserializer, Deserialize, Serialize, Serializer};
use sp_core::H256;

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::string::{String, ToString};
#[cfg(feature = "std")]
use std::string::{String, ToString};

use utils::Felt;

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
    pub root: Felt,
    pub prev_digest: Felt,
    pub digest: Felt,
}

impl Block {
    pub fn new(block_number: u64, root: Felt) -> Self {
        let prev_digest = Default::default();
        let digest = Self::hash_payload(&block_number.into(), &root, &prev_digest);

        Self {
            block_number,
            root,
            prev_digest,
            digest,
        }
    }

    /// Creates a new block with the given block number, root, and previous digest.
    pub fn new_from_prev_digest(block_number: u64, root: Felt, prev_digest: Felt) -> Self {
        let digest = Self::hash_payload(&block_number.into(), &root, &prev_digest);

        Self {
            block_number,
            root,
            prev_digest,
            digest,
        }
    }

    /// Creates a new block with the given block number and digest, initializing root and prev_digest to default values.
    pub fn new_from_digest(block_number: u64, digest: Felt) -> Self {
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

    pub fn digest(&self) -> Felt {
        self.digest
    }

    pub fn prev_digest(&self) -> Felt {
        self.prev_digest
    }

    pub fn try_from_previous(prev: &Self, block: Self) -> Result<Self, BlockError> {
        if block.block_number != prev.block_number + 1 {
            return Err(BlockError::BlockNumberMismatch(block.block_number));
        }
        let digest = Self::hash_payload(&block.block_number.into(), &block.root, &prev.digest);

        Ok(Self {
            block_number: block.block_number,
            root: block.root,
            prev_digest: prev.digest,
            digest,
        })
    }

    pub fn from_block_number_and_digest(block_number: u64, digest: Felt) -> Self {
        Self {
            block_number,
            digest,
            ..Default::default()
        }
    }

    pub fn hash_payload(block_number: &Felt, root: &Felt, prev_digest: &Felt) -> Felt {
        let d = starknet_crypto::pedersen_hash(block_number, root);
        starknet_crypto::pedersen_hash(&d, prev_digest)
    }
}

impl MaybeCreatedFromEmpty for Block {
    fn created_from_empty(&self) -> bool {
        self.root == Default::default()
    }
}

// Helper serde adapters to encode/decode H256 as decimal strings (via Felt) in JSON
fn h256_serialize_as_decimal<S>(val: &H256, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let felt = Felt::from_bytes_be(&val.0);
    s.serialize_str(&felt.to_string())
}

fn h256_deserialize_from_decimal<'de, D>(d: D) -> Result<H256, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    // Only accept decimal strings here
    match Felt::from_dec_str(&s) {
        Ok(f) => Ok(H256::from_slice(&f.to_bytes_be())),
        Err(_) => Err(serde::de::Error::custom("Felt decimal parse error")),
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
    block_number: u64,
    #[serde(
        serialize_with = "h256_serialize_as_decimal",
        deserialize_with = "h256_deserialize_from_decimal"
    )]
    root: H256,
    #[serde(
        serialize_with = "h256_serialize_as_decimal",
        deserialize_with = "h256_deserialize_from_decimal"
    )]
    prev_digest: H256,
    #[serde(
        serialize_with = "h256_serialize_as_decimal",
        deserialize_with = "h256_deserialize_from_decimal"
    )]
    digest: H256,
}

#[derive(Debug, Clone, Default)]
pub struct ContinuityBlock {
    root: Felt,
    digest: Felt,
}

#[derive(
    Debug, Clone, TypeInfo, Decode, Encode, PartialEq, Eq, Default, Serialize, Deserialize,
)]
pub struct ContinuityBlockSerializable {
    #[serde(
        serialize_with = "h256_serialize_as_decimal",
        deserialize_with = "h256_deserialize_from_decimal"
    )]
    root: H256,
    #[serde(
        serialize_with = "h256_serialize_as_decimal",
        deserialize_with = "h256_deserialize_from_decimal"
    )]
    digest: H256,
}

impl From<&ContinuityBlock> for ContinuityBlockSerializable {
    fn from(b: &ContinuityBlock) -> Self {
        Self {
            root: H256::from_slice(&b.root.to_bytes_be()),
            digest: H256::from_slice(&b.digest.to_bytes_be()),
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

    fn try_from(block: ContinuityBlockSerializable) -> Result<Self, ()> {
        Ok(Self {
            root: Felt::from_bytes_be(&block.root.0),
            digest: Felt::from_bytes_be(&block.digest.0),
        })
    }
}

impl From<&Block> for BlockSerializable {
    fn from(b: &Block) -> Self {
        Self {
            block_number: b.block_number,
            root: H256::from_slice(&b.root.to_bytes_be()),
            prev_digest: H256::from_slice(&b.prev_digest.to_bytes_be()),
            digest: H256::from_slice(&b.digest.to_bytes_be()),
        }
    }
}

impl From<BlockSerializable> for Block {
    fn from(val: BlockSerializable) -> Self {
        Block {
            block_number: val.block_number,
            root: Felt::from_bytes_be(&val.root.0),
            prev_digest: Felt::from_bytes_be(&val.prev_digest.0),
            digest: Felt::from_bytes_be(&val.digest.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to make a Felt from decimal for readable fixtures
    fn felt_dec(s: &str) -> Felt {
        Felt::from_dec_str(s).expect("test fixture Felt parsing failed")
    }

    #[test]
    fn continuity_block_serialization_is_decimal_and_roundtrips() {
        // Arrange
        let original = ContinuityBlock {
            root: felt_dec("123456789012345678901234567890"),
            digest: felt_dec("42"),
        };

        // Act: produce serializable and serialize to JSON
        let ser = ContinuityBlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");

        // Assert: serialized strings are decimal (no 0x prefix)
        assert!(
            !json.contains("0x") && !json.contains("0X"),
            "Serialized ContinuityBlockSerializable fields should be decimal strings without 0x prefix"
        );

        // Act: parse back to ContinuityBlockSerializable via JSON and then convert
        let parsed_ser: ContinuityBlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = ContinuityBlock::try_from(parsed_ser)
            .expect("parsing ContinuityBlockSerializable failed");

        // Assert: values round-trip (compare via to_string to avoid needing PartialEq on Felt)
        assert_eq!(original.root.to_string(), parsed.root.to_string());
        assert_eq!(original.digest.to_string(), parsed.digest.to_string());
    }

    #[test]
    fn block_serialization_is_decimal_and_roundtrips() {
        // Arrange: a Block with decimal Felt values
        let original = Block {
            block_number: 7,
            root: felt_dec("98765432109876543210"),
            prev_digest: felt_dec("3141592653589793238462643383279502884"),
            digest: felt_dec("2718281828459045235360287471352662497"),
        };

        // Act: produce serializable and serialize to JSON
        let ser = BlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");

        // Assert: serialized strings are decimal (no 0x prefix)
        assert!(!json.contains("0x") && !json.contains("0X"));

        // Act: parse back via JSON and convert to Block
        let parsed_ser: BlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = Block::from(parsed_ser);

        // Assert: fields match via string form
        assert_eq!(original.block_number, parsed.block_number);
        assert_eq!(original.root.to_string(), parsed.root.to_string());
        assert_eq!(
            original.prev_digest.to_string(),
            parsed.prev_digest.to_string()
        );
        assert_eq!(original.digest.to_string(), parsed.digest.to_string());
    }

    #[test]
    fn very_large_decimal_values_roundtrip() {
        // A 256-bit-ish large decimal to smoke-test boundaries (fits as Felt if your field allows it)
        let big = "1157920892373161954235709850086879078532699846656405640394575840079131296399"; // ~2^256-1

        let original = ContinuityBlock {
            root: felt_dec(big),
            digest: felt_dec("340282366920938463463374607431768211455"), // ~2^128-1
        };

        let ser = ContinuityBlockSerializable::from(&original);
        let json = serde_json::to_string(&ser).expect("serialization failed");
        assert!(!json.contains("0x") && !json.contains("0X"));

        let parsed_ser: ContinuityBlockSerializable =
            serde_json::from_str(&json).expect("deserialization failed");
        let parsed = ContinuityBlock::try_from(parsed_ser).expect("parsing large decimals failed");
        assert_eq!(original.root.to_string(), parsed.root.to_string());
        assert_eq!(original.digest.to_string(), parsed.digest.to_string());
    }
}
