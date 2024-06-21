use crate::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
use crate::block::{Block, BlockError, BlockSerializable};
use crate::FRAGMENT_SIZE;
use ethereum_types::U256;
use serde::{Deserialize, Serialize};
use utils::json_serializable::JsonSerializable;

#[derive(Debug, Clone)]
pub struct AttestationFragment {
    blocks: [Block; FRAGMENT_SIZE],
    len: usize,
}

impl AttestationFragment {
    pub fn new() -> Self {
        Self {
            //            blocks: [Default::default(); FRAGMENT_SIZE],
            blocks: core::array::from_fn(|_| Default::default()),
            len: 0,
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks[0..self.len]
    }

    pub fn head(&self) -> Option<&Block> {
        if self.len == 0 {
            None
        } else {
            Some(&self.blocks[self.len - 1])
        }
    }

    pub fn tail(&self) -> Option<&Block> {
        if self.is_empty() {
            None
        } else {
            Some(&self.blocks[0])
        }
    }
    pub fn checkpoint(&self) -> Option<AttestationCheckpoint> {
        if self.is_full() {
            self.head().map(AttestationCheckpoint::from)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn is_full(&self) -> bool {
        self.len() == FRAGMENT_SIZE
    }
    pub fn interval(&self) -> Option<AttestationInterval> {
        self.tail()
            .and_then(|tail| AttestationInterval::interval_for(tail.n() + 1))
    }
    pub fn next(&self) -> Option<Self> {
        if self.is_full() {
            let mut next = Self::default();
            next.try_append_block(self.blocks[FRAGMENT_SIZE - 1].clone())
                .expect("can append block to empty fragment");
            Some(next)
        } else {
            None
        }
    }

    pub fn try_append_block(&mut self, block: Block) -> Result<&Block, AttestationFragmentError> {
        if self.is_full() {
            return Err(AttestationFragmentError::FragmentIsFull);
        }
        if self.is_empty() && !AttestationInterval::is_aligned(block.n()) {
            return Err(AttestationFragmentError::MisalignedBlock(Box::new(block)));
        }

        let block = self
            .head()
            .map(|head| Block::try_from_previous(head, block.clone()))
            .unwrap_or(Ok(block))?;
        let head_digest = self.head().map(|head| head.digest());

        if head_digest == Some(block.prev_digest()) || head_digest.is_none() {
            self.blocks[self.len] = block;
            self.len += 1;
            Ok(&self.blocks[self.len - 1])
        } else {
            Err(AttestationFragmentError::BlockDigestMismatch(Box::new(
                block,
            )))
        }
    }

    pub fn attestation_slice_for(
        &self,
        block_number: U256,
        upper_bound: Option<U256>,
    ) -> Option<FragmentSlice> {
        let tail = self.tail().map(Block::n)?;
        let head = self.head().map(Block::n)?;
        let upper_bound = std::cmp::min(head, upper_bound.unwrap_or(head));

        if tail < block_number && head >= block_number {
            Some(FragmentSlice(
                &self.blocks
                    [(block_number - tail - 1).as_usize()..(upper_bound + 1 - tail).as_usize()],
            ))
        } else {
            None
        }
    }
}

impl AttestationFragment {
    pub fn try_from_file(fname: &str) -> Result<Self, AttestationFragmentError> {
        AttestationFragmentSerializable::try_from_file(fname)
            .map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))
            .and_then(AttestationFragment::try_from)
    }

    pub fn to_file(&self, fname: &str) -> Result<(), AttestationFragmentError> {
        AttestationFragmentSerializable::from(self)
            .to_file(fname)
            .map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))
    }
}
impl Default for AttestationFragment {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct FragmentSlice<'a>(&'a [Block]);

impl<'a> FragmentSlice<'a> {
    pub fn start(&self) -> Option<U256> {
        self.0.first().map(Block::n)
    }
    pub fn checkpoint(&self) -> Option<AttestationCheckpoint> {
        self.0.last().map(AttestationCheckpoint::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentSliceSerializable<'a> {
    pub blocks: Vec<BlockSerializable<'a>>,
}

impl<'a> From<FragmentSlice<'a>> for FragmentSliceSerializable<'a> {
    fn from(slice: FragmentSlice<'a>) -> Self {
        Self {
            blocks: slice.0.iter().map(BlockSerializable::from).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AttestationFragmentError {
    //    BlockNumberMismatch(u64),
    BlockNumberMismatch(U256),
    BlockDigestMismatch(Box<Block>),
    MisalignedBlock(Box<Block>),
    EmptyBlock(U256),
    FragmentIsFull,

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

impl<'a> TryFrom<AttestationFragmentSerializable<'a>> for AttestationFragment {
    type Error = AttestationFragmentError;

    fn try_from(chain_json: AttestationFragmentSerializable) -> Result<Self, Self::Error> {
        let mut chain = Self::new();

        for b in chain_json.blocks.into_iter().map(Block::try_from) {
            let b = b.map_err(|err| AttestationFragmentError::Other(format!("{err:?}")))?;

            chain.try_append_block(b)?;
        }
        Ok(chain)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationFragmentSerializable<'a> {
    blocks: Vec<BlockSerializable<'a>>,
}

impl JsonSerializable for AttestationFragmentSerializable<'_> {}

impl<'a> From<&'a AttestationFragment> for AttestationFragmentSerializable<'a> {
    fn from(fragment: &'a AttestationFragment) -> Self {
        Self {
            blocks: fragment
                .blocks()
                .iter()
                .map(BlockSerializable::from)
                .collect(),
        }
    }
}
