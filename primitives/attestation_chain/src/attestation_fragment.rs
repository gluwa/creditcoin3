use crate::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
use crate::block::{Block, BlockError, BlockSerializable};
use crate::AttestationChainParams;
use serde::{Deserialize, Serialize};
use utils::json_serializable::JsonSerializable;

#[derive(Debug, Clone)]
pub struct AttestationFragment {
    params: AttestationChainParams,
    blocks: Vec<Block>,
}

impl AttestationFragment {
    pub fn new(params: AttestationChainParams) -> Self {
        Self {
            params,
            blocks: Vec::with_capacity(params.fragment_size()),
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
        self.len() == self.params.fragment_size()
    }
    pub fn interval(&self) -> Option<AttestationInterval> {
        self.tail()
            .and_then(|tail| self.params.interval_for(tail.n() + 1))
    }

    pub fn next(&self) -> Option<Self> {
        self.is_full().then(|| {
            let mut next = Self::new(self.params);
            next.try_append_block(self.blocks[self.params.fragment_size() - 1].clone())
                .expect("can append block to empty fragment");
            next
        })
    }

    pub fn try_append_block(&mut self, block: Block) -> Result<&Block, AttestationFragmentError> {
        if self.is_full() {
            return Err(AttestationFragmentError::FragmentIsFull);
        }
        if self.is_empty() && !self.params.is_aligned(block.n()) {
            return Err(AttestationFragmentError::MisalignedBlock(Box::new(block)));
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

    pub fn attestation_slice_for(
        &self,
        block_number: u64,
        upper_bound: Option<u64>,
    ) -> Option<FragmentSlice> {
        let tail = self.tail().map(Block::n)?;
        let head = self.head().map(Block::n)?;
        let upper_bound = core::cmp::min(head, upper_bound.unwrap_or(head));

        (tail < block_number && head >= block_number).then_some(FragmentSlice(
            &self.blocks[(block_number - tail - 1) as usize..(upper_bound + 1 - tail) as usize],
        ))
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

#[derive(Debug, Clone)]
pub struct FragmentSlice<'a>(&'a [Block]);

impl<'a> FragmentSlice<'a> {
    pub fn start(&self) -> Option<u64> {
        self.0.first().map(Block::n)
    }
    pub fn checkpoint(&self) -> Option<AttestationCheckpoint> {
        self.0.last().map(AttestationCheckpoint::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentSliceSerializable {
    pub blocks: Vec<BlockSerializable>,
}

impl<'a> From<FragmentSlice<'a>> for FragmentSliceSerializable {
    fn from(slice: FragmentSlice<'a>) -> Self {
        Self {
            blocks: slice.0.iter().map(BlockSerializable::from).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AttestationFragmentError {
    //    BlockNumberMismatch(u64),
    BlockNumberMismatch(u64),
    BlockDigestMismatch(Box<Block>),
    MisalignedBlock(Box<Block>),
    EmptyBlock(u64),
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

impl TryFrom<(AttestationFragmentSerializable, AttestationChainParams)> for AttestationFragment {
    type Error = AttestationFragmentError;

    fn try_from(
        chain_json_with_params: (AttestationFragmentSerializable, AttestationChainParams),
    ) -> Result<Self, Self::Error> {
        let mut chain = Self::new(chain_json_with_params.1);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationFragmentSerializable {
    //    params: AttestationChainParams,
    blocks: Vec<BlockSerializable>,
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
