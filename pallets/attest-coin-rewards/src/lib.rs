//! Attest-coin reward points: accrue per **stash** each Babe-aligned epoch, claim on EVM via precompile.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{SaturatedConversion, Zero};
    use sp_std::prelude::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_attestation::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Reward points (same 1e18 precision as the ERC-20). Named separately from `Balances::Balance`.
        type RewardPoints: Parameter
            + Member
            + MaybeSerializeDeserialize
            + Copy
            + sp_std::fmt::Debug
            + Default
            + From<u128>
            + Into<u128>
            + core::ops::AddAssign
            + core::ops::SubAssign
            + PartialOrd
            + Zero
            + MaxEncodedLen;

        /// Must match `pallet_babe` / session epoch length in blocks (see runtime `EpochDuration`).
        #[pallet::constant]
        type EpochDuration: Get<BlockNumberFor<Self>>;

        /// Total points minted into `Accrued` per epoch, split equally across all bonded stashes.
        #[pallet::constant]
        type EpochRewardPool: Get<Self::RewardPoints>;
    }

    #[pallet::storage]
    pub type Accrued<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, T::RewardPoints, ValueQuery>;

    /// ERC-20 contract address (Option A minter is the attest-coin precompile, not this account).
    #[pallet::storage]
    pub type AttestCoinErc20<T: Config> = StorageValue<_, sp_core::H160, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Reward points credited after an epoch boundary.
        EpochRewardsAccrued {
            epoch_block: BlockNumberFor<T>,
            stashes: u32,
            per_stash: T::RewardPoints,
        },
        /// ERC-20 token address configured (governance).
        AttestCoinTokenSet { token: sp_core::H160 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// No ERC-20 configured yet.
        TokenNotConfigured,
        /// Not a bonded attestor stash.
        NotStash,
        /// Claim exceeds accrued points.
        InsufficientAccrued,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Set the attest-coin ERC-20 contract. **Root only** (governance / sudo).
        #[pallet::weight(Weight::from_parts(25_000, 0))]
        #[pallet::call_index(0)]
        pub fn set_attest_coin_token(origin: OriginFor<T>, token: sp_core::H160) -> DispatchResult {
            ensure_root(origin)?;
            AttestCoinErc20::<T>::put(token);
            Self::deposit_event(Event::AttestCoinTokenSet { token });
            Ok(())
        }

        /// Force one settlement (for tests / ops). **Root only.**
        #[pallet::weight(Weight::from_parts(50_000_000, 0))]
        #[pallet::call_index(1)]
        pub fn force_settle(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            Self::do_settlement(frame_system::Pallet::<T>::block_number());
            Ok(())
        }
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_finalize(n: BlockNumberFor<T>) {
            let epoch = T::EpochDuration::get();
            if epoch.is_zero() {
                return;
            }
            let n_u64: u64 = n.saturated_into();
            let e_u64: u64 = epoch.saturated_into();
            if e_u64 == 0 {
                return;
            }
            if n_u64 % e_u64 != 0 {
                return;
            }
            Self::do_settlement(n);
        }
    }

    impl<T: Config> Pallet<T> {
        /// Equal split of `EpochRewardPool` across every stash with an attestation ledger entry.
        pub fn do_settlement(at_block: BlockNumberFor<T>) {
            let stashes: Vec<T::AccountId> = pallet_attestation::Ledger::<T>::iter_keys().collect();
            let n = stashes.len() as u128;
            if n == 0 {
                return;
            }
            let pool: u128 = T::EpochRewardPool::get().into();
            let per_u128 = pool.saturating_div(n);
            let per = T::RewardPoints::from(per_u128);
            for stash in stashes.iter() {
                Accrued::<T>::mutate(stash, |a| *a += per);
            }
            Self::deposit_event(Event::EpochRewardsAccrued {
                epoch_block: at_block,
                stashes: stashes.len() as u32,
                per_stash: per,
            });
        }

        /// Deduct accrued points after a successful EVM mint (called from precompile).
        pub fn take_accrued_for_claim(
            stash: &T::AccountId,
            amount: T::RewardPoints,
        ) -> Result<(), Error<T>> {
            ensure!(
                pallet_attestation::Ledger::<T>::contains_key(stash),
                Error::<T>::NotStash
            );
            Accrued::<T>::try_mutate(stash, |acc| {
                if *acc < amount {
                    return Err(Error::<T>::InsufficientAccrued);
                }
                *acc -= amount;
                Ok(())
            })
        }

        pub fn accrued_of(stash: &T::AccountId) -> T::RewardPoints {
            Accrued::<T>::get(stash)
        }

        pub fn erc20_token() -> Option<sp_core::H160> {
            AttestCoinErc20::<T>::get()
        }

        /// Restore accrued points after a failed EVM mint (precompile rollback).
        pub fn restore_accrued(stash: &T::AccountId, amount: T::RewardPoints) {
            Accrued::<T>::mutate(stash, |a| *a += amount);
        }
    }
}
