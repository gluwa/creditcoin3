pub mod json_db;

use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::attestation_fragment::{AttestationFragment, AttestationFragmentError};
use attestation_chain::block::Block;
use attestation_chain::{ATTESTATION_GENESIS, CHECKPOINT_INTERVAL};
use ethereum_types::U256;
pub struct FullFragment<'a>(&'a AttestationFragment);

impl<'a> FullFragment<'a> {
    pub fn unwrap_fragment(self) -> AttestationFragment {
        self.0.clone()
    }
    pub fn inner(&self) -> &AttestationFragment {
        self.0
    }
}

impl<'a> TryFrom<&'a AttestationFragment> for FullFragment<'a> {
    type Error = ();

    fn try_from(fragment: &'a AttestationFragment) -> Result<Self, Self::Error> {
        if fragment.is_full() {
            Ok(Self(fragment))
        } else {
            Err(())
        }
    }
}

#[derive(Debug)]
pub enum AttestationDbError {
    FragmentAlreadySet(AttestationInterval),
    FragmentAfterRecent(AttestationInterval),

    MisalignedBlockDiscarded(Box<Block>),
    BlockNumberMismatch(U256),
    //    BlockNumberMismatch(u64),
    BlockDigestMismatch(Box<Block>),

    FragmentIsFull,
    ResetFailure,
    Other(String),
}

impl From<AttestationFragmentError> for AttestationDbError {
    fn from(err: AttestationFragmentError) -> AttestationDbError {
        match err {
            AttestationFragmentError::BlockNumberMismatch(block_number) => {
                AttestationDbError::BlockNumberMismatch(block_number)
            }
            AttestationFragmentError::BlockDigestMismatch(block) => {
                AttestationDbError::BlockDigestMismatch(block)
            }
            AttestationFragmentError::MisalignedBlock(block) => {
                AttestationDbError::MisalignedBlockDiscarded(block)
            }
            AttestationFragmentError::FragmentIsFull => AttestationDbError::FragmentIsFull,
            AttestationFragmentError::Other(msg) => AttestationDbError::Other(msg),
            _ => AttestationDbError::Other(format!("unexpected fragment error: {err:?}")),
        }
    }
}

#[allow(private_bounds)]
pub trait AttestationDB: AttestationDBImpl {
    fn checkpoint_interval(&self) -> usize;
    fn genesis(&self) -> U256;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn reset(&mut self) -> Result<(), AttestationDbError>;

    fn recent_fragment(&self) -> &AttestationFragment;

    fn get_fragment_for(&self, block_number: U256) -> Option<AttestationFragment>;

    fn fragment_for_exists(&self, block_number: U256) -> bool;
    fn fragment_exists(&self, interval: &AttestationInterval) -> bool;

    fn key_for(block_number: U256) -> Option<U256> {
        block_number
            .checked_sub(1u64.into())?
            .checked_sub(ATTESTATION_GENESIS)
            .map(|d| d / CHECKPOINT_INTERVAL as u64)
    }
    fn set_fragment(&mut self, full_fragment: FullFragment) -> Result<(), AttestationDbError> {
        let fragment = full_fragment.unwrap_fragment();

        let next = if self.recent_fragment().is_empty() {
            fragment.next()
        } else {
            if self.recent_fragment().tail().map(Block::n) <= fragment.tail().map(Block::n) {
                return Err(AttestationDbError::FragmentAfterRecent(
                    fragment.interval().expect("full fragment defines interval"),
                ));
            }
            None
        };

        self.commit(fragment)?;

        if let Some(next) = next {
            *self.recent_fragment_mut() = next;
        }
        Ok(())
    }

    fn try_append_block(
        &mut self,
        block: Block,
    ) -> Result<Option<Box<AttestationFragment>>, AttestationDbError> {
        self.recent_fragment_mut()
            .try_append_block(block.clone())
            .map_err(AttestationDbError::from)?;

        match self.recent_fragment().next() {
            Some(next_fragment) => {
                let prev_fragment = self.recent_fragment().clone();
                let res = self.commit(prev_fragment);
                *self.recent_fragment_mut() = next_fragment;
                res.map(Some)
            }
            None => Ok(None),
        }
    }
}

trait AttestationDBImpl {
    fn commit(
        &mut self,
        fragment: AttestationFragment,
    ) -> Result<Box<AttestationFragment>, AttestationDbError>;
    fn recent_fragment_mut(&mut self) -> &mut AttestationFragment;
}
