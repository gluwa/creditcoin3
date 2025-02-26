use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utils::Felt;

pub trait MaybeCreatedFromEmpty {
    fn created_from_empty(&self) -> bool;
}

#[derive(Debug, Error)]
pub enum BlockError {
    #[error("Block number mismatch: {0}")]
    BlockNumberMismatch(u64),
    #[error("Block: {0} was created from empty")]
    Empty(u64),
}

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

    pub fn new_from_prev(block_number: u64, root: Felt, prev_digest: Felt) -> Self {
        let digest = Self::hash_payload(&block_number.into(), &root, &prev_digest);

        Self {
            block_number,
            root,
            prev_digest,
            digest,
        }
    }

    pub fn new_with_digest(block_number: u64, root: Felt, digest: Felt) -> Self {
        let prev_digest = Default::default();

        Self {
            block_number,
            root,
            prev_digest,
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

#[derive(Encode, Decode, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockSerializable {
    block_number: u64,
    root: String,
    prev_digest: String,
    digest: String,
}

#[derive(Debug, Clone, Default)]
pub struct ContinuityBlock {
    root: Felt,
    digest: Felt,
}

#[derive(Encode, Decode, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContinuityBlockSerializable {
    root: String,
    digest: String,
}

impl From<&ContinuityBlock> for ContinuityBlockSerializable {
    fn from(b: &ContinuityBlock) -> Self {
        Self {
            root: b.root.to_string(),
            digest: b.digest.to_string(),
        }
    }
}

impl From<&Block> for ContinuityBlock {
    fn from(b: &Block) -> Self {
        Self {
            root: b.root.clone(),
            digest: b.digest.clone(),
        }
    }
}

impl From<BlockSerializable> for ContinuityBlockSerializable {
    fn from(b: BlockSerializable) -> Self {
        Self {
            root: b.root.clone(),
            digest: b.digest.clone(),
        }
    }
}

impl TryFrom<ContinuityBlockSerializable> for ContinuityBlock {
    type Error = ();

    fn try_from(block: ContinuityBlockSerializable) -> Result<Self, ()> {
        Ok(Self {
            root: Felt::from_dec_str(block.root.as_ref()).map_err(|_| ())?,
            digest: Felt::from_dec_str(block.digest.as_ref()).map_err(|_| ())?,
        })
    }
}

impl From<&Block> for BlockSerializable {
    fn from(b: &Block) -> Self {
        Self {
            block_number: b.block_number,
            root: b.root.to_string(),
            prev_digest: b.prev_digest.to_string(),
            digest: b.digest.to_string(),
        }
    }
}

impl TryFrom<BlockSerializable> for Block {
    type Error = ();

    fn try_from(block: BlockSerializable) -> Result<Self, ()> {
        Ok(Self {
            block_number: block.block_number,
            root: Felt::from_dec_str(block.root.as_ref()).map_err(|_| ())?,
            prev_digest: Felt::from_dec_str(block.prev_digest.as_ref()).map_err(|_| ())?,
            digest: Felt::from_dec_str(block.digest.as_ref()).map_err(|_| ())?,
        })
    }
}
