use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_std::vec::Vec;

use utils::Felt;

use crate::block::{Block, BlockError, BlockSerializable, ContinuityBlockSerializable};

#[derive(Debug, Clone, Default)]
pub struct AttestationFragment {
    fragment_length: usize,
    blocks: Vec<Block>,
}

impl AttestationFragment {
    pub fn new(fragment_length: usize) -> Self {
        Self {
            fragment_length,
            blocks: Vec::with_capacity(fragment_length),
        }
    }

    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        Self {
            fragment_length: blocks.len(),
            blocks,
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn head(&self) -> Option<&Block> {
        self.blocks.last()
    }

    pub fn head_digest(&self) -> Option<&Felt> {
        self.head().map(|block| &block.digest)
    }

    pub fn tail(&self) -> Option<&Block> {
        self.blocks.first()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn is_full(&self) -> bool {
        self.len() == self.fragment_length
    }

    // TODO: Appears to be completely vestigial. Only used in early prototype crates.
    // Not used in the current prover. Remove after those crates are cleaned up.
    pub fn next(&self) -> Option<Self> {
        self.is_full().then(|| {
            let mut next = Self::new(self.fragment_length);
            next.try_append_block(self.blocks[self.fragment_length - 1].clone())
                .expect("can append block to empty fragment");
            next
        })
    }

    pub fn try_append_block(&mut self, block: Block) -> Result<&Block, AttestationFragmentError> {
        if self.is_full() {
            return Err(AttestationFragmentError::FragmentIsFull);
        }

        let block = self
            .head()
            .map(|head| Block::try_from_previous(head, block.clone()))
            .unwrap_or(Ok(block))?;

        let head_digest = self.head().map(|head| head.digest());

        if head_digest == Some(block.prev_digest()) || head_digest.is_none() {
            self.blocks.push(block);
            Ok(self.head().expect("fragment not empty"))
        } else {
            Err(AttestationFragmentError::BlockDigestMismatch(block))
        }
    }

    pub fn blocks_serializable(
        &self,
        claim_block_number: u64,
    ) -> Result<FragmentBlocksSerializable, AttestationFragmentError> {
        let tail = self.tail().map(Block::n);
        let head = self.head().map(Block::n);
        match (tail, head) {
            (Some(tail), Some(head)) if tail < claim_block_number && head >= claim_block_number => {
                // Head and tail were found, and the claim block number lies between them,
                // we can take a subset of the fragment blocks to save proving time and size.
                let blocks_subset = self.blocks
                    [(claim_block_number - tail - 1) as usize..(head + 1 - tail) as usize]
                    .iter()
                    .map(BlockSerializable::from)
                    .collect();
                Ok(FragmentBlocksSerializable {
                    start: claim_block_number - 1,
                    blocks: blocks_subset,
                })
            }
            _ => Err(AttestationFragmentError::Other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FragmentBlocksSerializable {
    pub start: u64,
    pub blocks: Vec<BlockSerializable>,
}

impl From<FragmentBlocksSerializable> for FragmentContinuityBlocksSerializable {
    fn from(blocks: FragmentBlocksSerializable) -> Self {
        Self {
            start: blocks.start,
            blocks: blocks
                .blocks
                .into_iter()
                .map(ContinuityBlockSerializable::from)
                .collect::<Vec<_>>(),
        }
    }
}

#[derive(Debug, Clone, TypeInfo, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
pub struct FragmentContinuityBlocksSerializable {
    pub start: u64,
    pub blocks: Vec<ContinuityBlockSerializable>,
}

#[derive(Debug, Clone)]
pub enum AttestationFragmentError {
    BlockNumberMismatch(u64),
    BlockDigestMismatch(Block),
    MisalignedBlock(Block),
    EmptyBlock(u64),
    FragmentIsFull,
    Other,
}

#[cfg(feature = "std")]
impl core::fmt::Display for AttestationFragmentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BlockNumberMismatch(num) => write!(f, "Block number mismatch: expected {num}"),
            Self::BlockDigestMismatch(block) => write!(f, "Block digest mismatch: {block:?}"),
            Self::MisalignedBlock(block) => write!(f, "Misaligned block: {block:?}"),
            Self::EmptyBlock(num) => write!(f, "Empty block at height {num}"),
            Self::FragmentIsFull => write!(f, "Fragment is full"),
            Self::Other => write!(f, "An unknown error occurred"),
        }
    }
}

#[cfg(feature = "std")]
impl core::error::Error for AttestationFragmentError {}

impl From<BlockError> for AttestationFragmentError {
    fn from(err: BlockError) -> AttestationFragmentError {
        match err {
            BlockError::BlockNumberMismatch(block_number) => {
                AttestationFragmentError::BlockNumberMismatch(block_number)
            }
            BlockError::Empty(block_number) => AttestationFragmentError::EmptyBlock(block_number),
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Encode, Decode, TypeInfo, Default, Serialize, Deserialize,
)]
pub struct AttestationFragmentSerializable {
    //    params: AttestationChainParams,
    blocks: Vec<BlockSerializable>,
}

impl AttestationFragmentSerializable {
    pub fn get_blocks_ref(&self) -> &Vec<BlockSerializable> {
        &self.blocks
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn head(&self) -> Option<&BlockSerializable> {
        self.blocks.last()
    }

    pub fn tail(&self) -> Option<&BlockSerializable> {
        self.blocks.first()
    }
}

impl From<&AttestationFragment> for AttestationFragmentSerializable {
    fn from(fragment: &AttestationFragment) -> Self {
        Self {
            blocks: fragment
                .blocks()
                .iter()
                .map(BlockSerializable::from)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use utils::Felt;

    use super::*;

    #[test]
    fn test_attestation_fragment() {
        let mut fragment = AttestationFragment::new(10);

        assert_eq!(fragment.blocks().len(), 0);
        assert!(!fragment.is_full());

        for i in 0..10 {
            let block = Block::new(i, Felt::default());
            let block = fragment.try_append_block(block).unwrap();
            assert_eq!(block.n(), i);
            assert_eq!(fragment.blocks().len(), (i + 1) as usize);
            assert_eq!(fragment.head().unwrap().n(), i);
            assert_eq!(fragment.tail().unwrap().n(), 0);
        }

        assert!(fragment.is_full());
    }

    #[test]
    fn serialize_attestation_fragment() {
        let mut fragment = AttestationFragment::new(10);

        for i in 0..10 {
            let block = Block::new(i, Felt::default());
            let block = fragment.try_append_block(block).unwrap();
            assert_eq!(block.n(), i);
        }

        let serializable = AttestationFragmentSerializable::from(&fragment);

        assert!(serializable.len() == 10);
    }
}
