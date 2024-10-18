use codec::{Decode, Encode};
use frame_support::traits::{LockIdentifier, LockableCurrency};
use parity_scale_codec::{self as codec, HasCompact, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_runtime::{BoundedVec, RuntimeDebug, Saturating};
use sp_std::vec::Vec;

use super::pallet::*;
use crate::{BalanceOf, Config, Error, RewardDestination};

pub const BOND_LOCK_ID: LockIdentifier = *b"b0ndl0ck";

pub type EraIndex = u32;

#[derive(RuntimeDebug, Clone, Encode, Decode, Default, TypeInfo, MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct AttestorLedger<T: Config> {
    /// The stash account whose balance is actually locked and at stake.
    pub stash: T::AccountId,
    #[codec(compact)]
    /// The total amount of the stash's balance that we are currently accounting for.
    /// It's just `active` plus all the `unlocking` balances.
    pub total_staked: BalanceOf<T>,
    #[codec(compact)]
    /// The total amount of the stash's balance that will be at stake in any forthcoming
    /// rounds.
    pub active: BalanceOf<T>,
    /// Any balance that is becoming free, which may eventually be transferred out of the stash
    /// (assuming it doesn't get slashed first). It is assumed that this will be treated as a first
    /// in, first out queue where the new (higher value) eras get pushed on the back.
    pub unlocking: BoundedVec<UnlockChunk<BalanceOf<T>>, T::MaxUnlockingChunks>,
}

/// Just a Balance/BlockNumber tuple to encode when a chunk of funds will be unlocked.
#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct UnlockChunk<Balance: HasCompact + MaxEncodedLen> {
    /// Amount of funds to be unlocked.
    #[codec(compact)]
    pub value: Balance,
    /// Era number at which point it'll be unlocked.
    #[codec(compact)]
    pub era: EraIndex,
}

#[cfg(any(feature = "runtime-benchmarks", test))]
use sp_runtime::traits::Zero;

impl<T: Config> AttestorLedger<T> {
    #[cfg(any(feature = "runtime-benchmarks", test))]
    pub fn default_from(stash: T::AccountId) -> Self {
        Self {
            stash: stash.clone(),
            total_staked: Zero::zero(),
            active: Zero::zero(),
            unlocking: BoundedVec::default(),
        }
    }

    pub fn new(stash: T::AccountId, stake: BalanceOf<T>) -> Self {
        Self {
            stash,
            total_staked: stake,
            active: stake,
            unlocking: BoundedVec::default(),
        }
    }

    /// Updates the ledger.
    ///
    /// It sets the lock for the bonded stash.
    pub(crate) fn update(self) -> Result<(), Error<T>> {
        if !<Ledger<T>>::contains_key(&self.stash) {
            return Err(Error::<T>::NotStash);
        }

        T::Currency::set_lock(
            BOND_LOCK_ID,
            &self.stash,
            self.total_staked,
            frame_support::traits::WithdrawReasons::all(),
        );

        Ledger::<T>::insert(&self.stash, &self);

        Ok(())
    }

    /// Bonds a ledger.
    ///
    /// It sets the reward preferences for the bonded stash.
    pub(crate) fn bond(self, payee: RewardDestination<T::AccountId>) -> Result<(), Error<T>> {
        if <Ledger<T>>::contains_key(&self.stash) {
            return Err(Error::<T>::AlreadyBonded);
        }

        <Ledger<T>>::insert(&self.stash, &self);
        <Payee<T>>::insert(&self.stash, payee);
        self.update()
    }

    /// Sets the ledger Payee.
    pub(crate) fn set_payee(self, payee: RewardDestination<T::AccountId>) -> Result<(), Error<T>> {
        if !<Ledger<T>>::contains_key(&self.stash) {
            return Err(Error::<T>::NotStash);
        }

        <Payee<T>>::insert(&self.stash, payee);
        Ok(())
    }

    /// Remove entries from `unlocking` that are sufficiently old and reduce the
    /// total by the sum of their balances.
    pub fn consolidate_unlocked(self, current_era: EraIndex) -> Self {
        let mut total = self.total_staked;
        let unlocking: BoundedVec<_, _> = self
            .unlocking
            .into_iter()
            .filter(|chunk| {
                if chunk.era > current_era {
                    true
                } else {
                    total = total.saturating_sub(chunk.value);
                    false
                }
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect(
                "filtering items from a bounded vec always leaves length less than bounds. qed",
            );

        Self {
            stash: self.stash,
            total_staked: total,
            active: self.active,
            unlocking,
        }
    }

    pub(crate) fn kill(stash: &T::AccountId) -> Result<(), Error<T>> {
        if !<Ledger<T>>::contains_key(stash) {
            return Err(Error::<T>::NotStash);
        }

        T::Currency::remove_lock(BOND_LOCK_ID, stash);
        <Ledger<T>>::remove(stash);
        <Payee<T>>::remove(stash);

        Ok(())
    }
}
