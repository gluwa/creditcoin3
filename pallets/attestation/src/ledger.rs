use codec::{Decode, Encode};
use frame_support::traits::{tokens::fungibles::Mutate, tokens::Preservation, Get};
use parity_scale_codec::{self as codec, HasCompact, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_runtime::{traits::Zero, BoundedVec, DispatchResult, RuntimeDebug, Saturating};
use sp_std::cmp::Ordering;
use sp_std::vec::Vec;

use super::pallet::*;
use crate::{BalanceOf, Config, Error};

pub type EraIndex = u32;

#[derive(RuntimeDebug, Clone, Encode, Decode, Default, TypeInfo, MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct AttestorLedger<T: Config> {
    /// The stash account whose bond is held in [`Config::BondPoolAccount`].
    pub stash: T::AccountId,
    #[codec(compact)]
    /// The total amount of the stash's bond that we are currently accounting for.
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

    /// Move bond delta between stash and the shared bond pool to match `self.total_staked`.
    pub(crate) fn sync_pool_delta(&self, old_total: BalanceOf<T>) -> DispatchResult {
        let new_total = self.total_staked;
        if new_total == old_total {
            return Ok(());
        }
        let id = T::BondAssetId::get();
        let pool = T::BondPoolAccount::get();
        match new_total.cmp(&old_total) {
            Ordering::Greater => {
                let d = new_total.saturating_sub(old_total);
                T::BondFungibles::transfer(id, &self.stash, &pool, d, Preservation::Expendable)?;
            }
            Ordering::Less => {
                let d = old_total.saturating_sub(new_total);
                T::BondFungibles::transfer(id, &pool, &self.stash, d, Preservation::Expendable)?;
            }
            Ordering::Equal => {}
        }
        Ok(())
    }

    /// Updates the ledger and syncs the bond pool.
    pub(crate) fn update(self) -> Result<(), Error<T>> {
        let old = <Ledger<T>>::get(&self.stash).ok_or(Error::<T>::NotStash)?;
        let old_total = old.total_staked;
        self.sync_pool_delta(old_total)
            .map_err(|_| Error::<T>::BondAssetTransferFailed)?;
        <Ledger<T>>::insert(&self.stash, &self);
        Ok(())
    }

    /// Bonds a ledger: moves initial stake into the bond pool.
    pub(crate) fn bond(self) -> Result<(), Error<T>> {
        if <Ledger<T>>::contains_key(&self.stash) {
            return Err(Error::<T>::AlreadyBonded);
        }
        self.sync_pool_delta(Zero::zero())
            .map_err(|_| Error::<T>::BondAssetTransferFailed)?;
        <Ledger<T>>::insert(&self.stash, &self);
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
        let ledger = <Ledger<T>>::take(stash).ok_or(Error::<T>::NotStash)?;
        if !ledger.total_staked.is_zero() {
            let id = T::BondAssetId::get();
            let pool = T::BondPoolAccount::get();
            T::BondFungibles::transfer(
                id,
                &pool,
                stash,
                ledger.total_staked,
                Preservation::Expendable,
            )
            .map_err(|_| Error::<T>::BondAssetTransferFailed)?;
        }
        Ok(())
    }
}
