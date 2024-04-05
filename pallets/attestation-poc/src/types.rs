use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::{ConstU32, RuntimeDebug};
use sp_runtime::BoundedVec;

const HASH_LEN: u32 = 76;

type TxRootLen = ConstU32<HASH_LEN>;
type RxRootLen = ConstU32<HASH_LEN>;
type PrevDigestLen = ConstU32<HASH_LEN>;
type DigestLen = ConstU32<HASH_LEN>;

pub type BlockNumber = u64;
pub type TxRoot = BoundedVec<u8, TxRootLen>;
pub type RxRoot = BoundedVec<u8, RxRootLen>;
pub type PrevDigest = BoundedVec<u8, PrevDigestLen>;
pub type Digest = BoundedVec<u8, DigestLen>;

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct BlockSerializable {
    pub block_number: BlockNumber,
    pub tx_root: TxRoot,
    pub rx_root: RxRoot,
    pub prev_digest: PrevDigest,
    pub digest: Digest,
}
