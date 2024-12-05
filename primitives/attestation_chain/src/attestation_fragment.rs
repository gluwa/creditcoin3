use crate::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
use crate::block::{Block, BlockError, BlockSerializable};
use crate::AttestationChainParams;
use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utils::json_serializable::JsonSerializable;

#[derive(Debug, Clone)]
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

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn head(&self) -> Option<&Block> {
        self.blocks.last()
    }

    pub fn tail(&self) -> Option<&Block> {
        self.blocks.first()
    }

    pub fn checkpoint(&self) -> Option<AttestationCheckpoint> {
        if self.is_full() {
            self.head().map(AttestationCheckpoint::from)
        } else {
            None
        }
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

    // With variable interval length, calculating the start of an interval is more
    // involved and costly. We do this once in get_interval_bounds() of fragment.rs
    // Here we assume that the fragment was filled with the appropriate blocks rather
    // than trying to re-calculate the interval from scratch.
    pub fn interval(&self) -> Option<AttestationInterval> {
        self.tail()
            .map(|tail| AttestationInterval(tail.n() + 1, tail.n() + self.fragment_length as u64))
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
            Err(AttestationFragmentError::BlockDigestMismatch(Box::new(
                block,
            )))
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
                    blocks: blocks_subset,
                })
            }
            _ => {
                Err(AttestationFragmentError::Other(format!("Could not get block subset from fragment. Fragment start: {:?}, Fragment end: {:?}, Claim block: {:?}", tail, head, claim_block_number)))
            }
        }
    }
}

impl AttestationFragment {
    pub fn try_from_file(
        fname: &str,
        params: AttestationChainParams,
    ) -> Result<Self, AttestationFragmentError> {
        AttestationFragmentSerializable::try_from_file(fname)
            .map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))
            .and_then(|fr| AttestationFragment::try_from((fr, params)))
    }

    pub fn to_file(&self, fname: &str) -> Result<(), AttestationFragmentError> {
        AttestationFragmentSerializable::from(self)
            .to_file(fname)
            .map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentBlocksSerializable {
    pub blocks: Vec<BlockSerializable>,
}

#[derive(Debug, Clone, Error)]
pub enum AttestationFragmentError {
    #[error("Fragment blocks must be sequential! Block number: {0}")]
    BlockNumberMismatch(u64),
    #[error("Prev digest of added block must match digest of prior block, Block: {0:?}")]
    BlockDigestMismatch(Box<Block>),
    #[error("Misaligned block, Block: {0:?}")]
    MisalignedBlock(Box<Block>),
    #[error("`root` field of block is empty, Block number: {0}")]
    EmptyBlock(u64),
    #[error("Cannot add block to full fragment")]
    FragmentIsFull,
    #[error("{0}")]
    Other(String),
}

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

impl TryFrom<(AttestationFragmentSerializable, AttestationChainParams)> for AttestationFragment {
    type Error = AttestationFragmentError;

    fn try_from(
        chain_json_with_params: (AttestationFragmentSerializable, AttestationChainParams),
    ) -> Result<Self, Self::Error> {
        let mut chain = Self::new(chain_json_with_params.1.interval);

        for b in chain_json_with_params
            .0
            .blocks
            .into_iter()
            .map(Block::try_from)
        {
            let b = b.map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))?;

            chain.try_append_block(b)?;
        }
        Ok(chain)
    }
}

#[derive(Encode, Decode, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttestationFragmentSerializable {
    //    params: AttestationChainParams,
    blocks: Vec<BlockSerializable>,
}

impl AttestationFragmentSerializable {
    pub fn get_blocks_ref(&self) -> &Vec<BlockSerializable> {
        &self.blocks
    }
}

impl JsonSerializable for AttestationFragmentSerializable {}

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
}
