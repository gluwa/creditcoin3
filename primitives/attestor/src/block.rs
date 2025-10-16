use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;

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
    root: H256,
    prev_digest: H256,
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
    root: H256,
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

impl From<&Block> for ContinuityBlock {
    fn from(b: &Block) -> Self {
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
