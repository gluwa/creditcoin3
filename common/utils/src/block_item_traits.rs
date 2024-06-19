use ethereum_types::U256;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::utils::U248_BYTE_COUNT;

pub trait CacheT<T>: Clone {
    type CachedItem: TryInto<T> + Serialize + for<'a> Deserialize<'a>;

    fn key(&self) -> &str;
    fn try_create_key(&mut self) -> anyhow::Result<()>;

    fn try_read(&self) -> anyhow::Result<Vec<Self::CachedItem>> {
        let file = std::fs::File::open(self.key())?;
        Ok(serde_json::from_reader::<_, Vec<Self::CachedItem>>(file)?)
    }

    fn try_write(&mut self, items: &[Self::CachedItem]) -> anyhow::Result<()> {
        self.try_create_key()?;

        let file = std::fs::File::create(self.key())?;

        Ok(serde_json::to_writer_pretty(file, items)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BlockItemIdentifier {
    block_number: U256,
    index: u64,
}

impl BlockItemIdentifier {
    pub fn new(block_number: U256, index: u64) -> Self {
        Self {
            block_number,
            index
        }
    }
    pub fn block_number(&self) -> U256 {
        self.block_number
    }
    #[inline(always)]
    pub fn index(&self) -> u64 {
        self.index
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        use std::mem::size_of;
        // bytes memory layout:
        // Merkle LEAF_HASH_PREPEND_VALUE                       -> u8                -> felt[0]
        // padding                                              -> 30 bytes
        // block_number                                         -> U256              -> felts[1,2] (block number hi & lo big endian)
        // shifting of index (u64) to fit big endian 31 bytes   -> (31 - 8) bytes    -> felt[3] (index u64 big endian)
        const BLOCK_HI_ALIGNMENT_PADDING_LEN: usize = U248_BYTE_COUNT - size_of::<u8>();
        // the hi part of 256-bit long block number to be shifted to right due to big-endianness 
        const BLOCK_HI_BE_SHIFT_LEN: usize = U248_BYTE_COUNT - (size_of::<U256>() - U248_BYTE_COUNT);
        const INDEX_BE_SHIFT_LEN: usize = U248_BYTE_COUNT - size_of::<u64>();

        let mut buf = [0u8;  size_of::<Self>() + BLOCK_HI_ALIGNMENT_PADDING_LEN + BLOCK_HI_BE_SHIFT_LEN + INDEX_BE_SHIFT_LEN];
        let block_number_offset = BLOCK_HI_ALIGNMENT_PADDING_LEN + BLOCK_HI_BE_SHIFT_LEN;
        self.block_number.to_big_endian(
        &mut buf[block_number_offset..block_number_offset + size_of::<U256>()]
        );

        let index_offset = block_number_offset + size_of::<U256>() + INDEX_BE_SHIFT_LEN;
        buf[index_offset..].copy_from_slice(&self.index.to_be_bytes());
        buf.to_vec()
    }
}
pub trait BlockItem: Sized {
    fn to_bytes(&self) -> Vec<u8>;

    fn id(&self) -> &BlockItemIdentifier;
    fn tx_type(&self) -> Option<u8>;
}
