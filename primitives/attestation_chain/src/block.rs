use ethereum_types::U256;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use utils::utils::u256_to_felts;
use utils::Felt;
pub trait MaybeCreatedFromEmpty {
    fn created_from_empty(&self) -> bool;
}

#[derive(Debug)]
pub enum BlockError {
    //    BlockNumberMismatch(u64),
    BlockNumberMismatch(U256),
    Empty(U256),
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub block_number: U256,
    pub tx_root: Felt,
    pub rx_root: Felt,
    pub prev_digest: Felt,
    pub digest: Felt,
}

impl Block {
    pub fn new(block_number: U256, tx_root: Felt, rx_root: Felt) -> Self {
        //        pub fn new(block_number: u64, tx_root: Felt, rx_root: Felt) -> Self {
        let prev_digest = Default::default();
        let (block_number_lo, block_number_hi) = u256_to_felts(&block_number);
        let digest = Self::hash_payload(
            // &block_number.into(),
            &block_number_lo,
            &block_number_hi,
            &tx_root,
            &rx_root,
            &prev_digest,
        );

        Self {
            block_number,
            tx_root,
            rx_root,
            prev_digest,
            digest,
        }
    }
    pub fn n(&self) -> U256 {
        //        let n = self.block_number.as_u64();
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
            //            if block.block_number != prev.block_number + 1u64 {
            return Err(BlockError::BlockNumberMismatch(block.block_number));
        }
        let (block_number_lo, block_number_hi) = u256_to_felts(&block.block_number);
        let digest = Self::hash_payload(
            &block_number_lo,
            &block_number_hi,
            //            &block.block_number.into(),
            &block.tx_root,
            &block.rx_root,
            &prev.digest,
        );

        Ok(Self {
            block_number: block.block_number,
            tx_root: block.tx_root,
            rx_root: block.rx_root,
            prev_digest: prev.digest,
            digest,
        })
    }
    pub fn from_block_number_and_digest(block_number: U256, digest: Felt) -> Self {
        Self {
            block_number,
            digest,
            ..Default::default()
        }
    }

    fn hash_payload(
        //        block_number: &Felt,
        block_number_lo: &Felt,
        block_number_hi: &Felt,
        tx_root: &Felt,
        rx_root: &Felt,
        prev_digest: &Felt,
    ) -> Felt {
        let d1 = starknet_crypto::pedersen_hash(block_number_lo, block_number_hi);
        let d2 = starknet_crypto::pedersen_hash(&d1, tx_root);
        let d3 = starknet_crypto::pedersen_hash(&d2, rx_root);
        starknet_crypto::pedersen_hash(&d3, prev_digest)
        // let d1 = starknet_crypto::pedersen_hash(block_number, tx_root);
        // let d2 = starknet_crypto::pedersen_hash(&d1, rx_root);
        // starknet_crypto::pedersen_hash(&d2, prev_digest)
    }
}

impl MaybeCreatedFromEmpty for Block {
    fn created_from_empty(&self) -> bool {
        self.tx_root == Default::default() && self.rx_root == Default::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSerializable<'a> {
    block_number: String,
    // block_number_lo: String,
    // block_number_hi: String,
    tx_root: String,
    rx_root: String,
    prev_digest: String,
    digest: String,
    #[serde(skip_serializing, skip_deserializing)]
    _marker: PhantomData<&'a ()>,
}

impl<'a> From<&'a Block> for BlockSerializable<'a> {
    fn from(b: &'a Block) -> Self {
        //        let (block_number_lo, block_number_hi) = u256_to_felts(&b.block_number);
        Self {
            block_number: b.block_number.to_string(),
            // block_number_lo: block_number_lo.to_string(),
            // block_number_hi: block_number_hi.to_string(),
            tx_root: b.tx_root.to_string(),
            rx_root: b.rx_root.to_string(),
            prev_digest: b.prev_digest.to_string(),
            digest: b.digest.to_string(),
            _marker: PhantomData,
        }
    }
}

impl TryFrom<BlockSerializable<'_>> for Block {
    type Error = ();

    fn try_from(block: BlockSerializable) -> Result<Self, ()> {
        // let block_number_lo = Felt::from_dec_str(block.block_number_lo.as_ref()).map_err(|_| ())?;
        // let block_number_hi = Felt::from_dec_str(block.block_number_hi.as_ref()).map_err(|_| ())?;
        // let block_number = u256_from_felts(&block_number_lo, &block_number_hi);
        Ok(Self {
            //            block_number: block.block_number.parse().map_err(|_| ())?,
            block_number: U256::from_dec_str(block.block_number.as_ref()).map_err(|_| ())?,
            tx_root: Felt::from_dec_str(block.tx_root.as_ref()).map_err(|_| ())?,
            rx_root: Felt::from_dec_str(block.rx_root.as_ref()).map_err(|_| ())?,
            prev_digest: Felt::from_dec_str(block.prev_digest.as_ref()).map_err(|_| ())?,
            digest: Felt::from_dec_str(block.digest.as_ref()).map_err(|_| ())?,
        })
    }
}
