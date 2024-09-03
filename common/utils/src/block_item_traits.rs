use core::fmt::Debug;
use serde::{Deserialize, Serialize};

use sp_std::vec::Vec;

use crate::utils::U248_BYTE_COUNT;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

#[derive(
    Debug,
    Default,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
pub struct BlockItemIdentifier {
    pub block_number: u64,
    pub index: u64,
}

impl BlockItemIdentifier {
    pub fn new(block_number: u64, index: u64) -> Self {
        Self {
            block_number,
            index,
        }
    }
    pub fn block_number(&self) -> u64 {
        self.block_number
    }
    #[inline(always)]
    pub fn index(&self) -> u64 {
        self.index
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        use core::mem::size_of;
        // bytes memory layout:
        // Merkle LEAF_HASH_PREPEND_VALUE                       -> u8                -> felt[0]
        // padding                                              -> 30 bytes
        // block_number                                         -> u64              -> felts[1,2] (block number hi & lo big endian)
        // shifting of index (u64) to fit big endian 31 bytes   -> (31 - 8) bytes    -> felt[3] (index u64 big endian)
        const INDEX_ALIGNMENT_PADDING_LEN: usize = U248_BYTE_COUNT - size_of::<u8>();
        // const BLOCK_HI_ALIGNMENT_PADDING_LEN: usize = U248_BYTE_COUNT - size_of::<u8>();
        // // the hi part of 256-bit long block number to be shifted to right due to big-endianness
        // const BLOCK_HI_BE_SHIFT_LEN: usize = U248_BYTE_COUNT - (size_of::<u64>() - U248_BYTE_COUNT);
        const INDEX_BE_SHIFT_LEN: usize = U248_BYTE_COUNT - size_of::<u64>();

        //        let mut buf = [0u8;  size_of::<Self>() + BLOCK_HI_ALIGNMENT_PADDING_LEN + BLOCK_HI_BE_SHIFT_LEN + INDEX_BE_SHIFT_LEN];
        let mut buf = [0u8; size_of::<u64>() + INDEX_ALIGNMENT_PADDING_LEN + INDEX_BE_SHIFT_LEN];
        // let block_number_offset = BLOCK_HI_ALIGNMENT_PADDING_LEN + BLOCK_HI_BE_SHIFT_LEN;
        // self.block_number.to_big_endian(
        // &mut buf[block_number_offset..block_number_offset + size_of::<u64>()]
        // );

        //        let index_offset = block_number_offset + size_of::<u64>() + INDEX_BE_SHIFT_LEN;
        let index_offset = INDEX_ALIGNMENT_PADDING_LEN + INDEX_BE_SHIFT_LEN;
        buf[index_offset..].copy_from_slice(&self.index.to_be_bytes());
        buf.to_vec()
    }
}
pub trait BlockItem: Sized {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.id().to_bytes();

        bytes.extend::<Vec<_>>(self.payload_bytes());
        bytes
    }

    fn id(&self) -> &BlockItemIdentifier;
    fn payload_bytes(&self) -> Vec<u8>;
    fn tx_type(&self) -> Option<u8>;
}
