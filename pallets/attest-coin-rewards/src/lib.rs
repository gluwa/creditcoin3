//! Attest-coin reward points: accrue per **stash** when attestors appear as **eligible signers**
//! on a committed attestation; optional **sudo** epoch split for tests; claim on EVM via precompile.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use attestor_primitives::ChainKey;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use parity_scale_codec::Encode;
    use sp_runtime::traits::Zero;
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

        /// Points credited to each **stash** backing an eligible signer on a successful
        /// [`pallet_attestation::Pallet::commit_attestation`].
        #[pallet::constant]
        type RewardPerEligibleSigner: Get<Self::RewardPoints>;

        /// Total points for [`Self::do_settlement`] (e.g. [`crate::Pallet::force_settle`]), split
        /// equally across every stash in [`pallet_attestation::Ledger`]. Not used by automatic
        /// block hooks.
        #[pallet::constant]
        type EpochRewardPool: Get<Self::RewardPoints>;
    }

    #[pallet::storage]
    pub type Accrued<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, T::RewardPoints, ValueQuery>;

    /// Monotonic claim nonce per stash (for sr25519-signed EVM claims).
    #[pallet::storage]
    pub type ClaimNonce<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

    /// ERC-20 contract address (treasury tokens sit here; claims use `transfer`, not `mint`).
    #[pallet::storage]
    pub type AttestCoinErc20<T: Config> = StorageValue<_, sp_core::H160, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Reward points credited to stashes after a committed attestation (one unit per eligible signer).
        CommitSignersRewarded {
            chain_key: ChainKey,
            signers: u32,
            per_signer: T::RewardPoints,
        },
        /// Reward points from [`Self::do_settlement`] (tests / manual ops).
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
        /// Not a bonded attestor stash (`pallet_attestation::Ledger`).
        NotStash,
        /// Claim exceeds accrued points.
        InsufficientAccrued,
        /// Claim nonce does not match on-chain counter.
        BadClaimNonce,
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

        /// Split [`Config::EpochRewardPool`] across all ledger stashes (tests / ops). **Root only.**
        #[pallet::weight(Weight::from_parts(50_000_000, 0))]
        #[pallet::call_index(1)]
        pub fn force_settle(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            Self::do_settlement(frame_system::Pallet::<T>::block_number());
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Credit [`Config::RewardPerEligibleSigner`] to each **stash** for the given **eligible**
        /// attestor operator accounts (from [`pallet_attestation`]). Called from the runtime hook
        /// after [`pallet_attestation::Pallet::commit_attestation`].
        pub fn reward_commit_signers(chain_key: ChainKey, eligible_signers: &[T::AccountId]) {
            let per = T::RewardPerEligibleSigner::get();
            if per.is_zero() || eligible_signers.is_empty() {
                return;
            }

            let mut credited = 0u32;
            for attestor_id in eligible_signers {
                if let Some(att) = pallet_attestation::Pallet::<T>::attestors(chain_key, attestor_id)
                {
                    let stash = att.stash;
                    Accrued::<T>::mutate(&stash, |a| *a += per);
                    credited = credited.saturating_add(1);
                }
            }

            if credited > 0 {
                Self::deposit_event(Event::CommitSignersRewarded {
                    chain_key,
                    signers: credited,
                    per_signer: per,
                });
            }
        }

        /// Equal split of [`Config::EpochRewardPool`] across every stash with an attestation ledger entry.
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

        /// Bytes that must be signed (sr25519) for [`Self::commit_claim`].
        ///
        /// Layout: `b"AttestCoin:claim:v1:" || stash_id(32) || nonce(le u64) || chain_key(le u64)
        /// || amount(le u128) || evm_recipient(20)`.
        pub fn claim_signing_message(
            stash: &T::AccountId,
            nonce: u64,
            chain_key: u64,
            amount: u128,
            evm_recipient: [u8; 20],
        ) -> Vec<u8> {
            const PREFIX: &[u8] = b"AttestCoin:claim:v1:";
            let mut m = Vec::with_capacity(PREFIX.len() + 32 + 8 + 8 + 16 + 20);
            m.extend_from_slice(PREFIX);
            let enc = stash.encode();
            let mut id = [0u8; 32];
            if enc.len() >= 32 {
                id.copy_from_slice(&enc[..32]);
            } else if !enc.is_empty() {
                id[32 - enc.len()..].copy_from_slice(&enc);
            }
            m.extend_from_slice(&id);
            m.extend_from_slice(&nonce.to_le_bytes());
            m.extend_from_slice(&chain_key.to_le_bytes());
            m.extend_from_slice(&amount.to_le_bytes());
            m.extend_from_slice(&evm_recipient);
            m
        }

        pub fn claim_nonce_of(stash: &T::AccountId) -> u64 {
            ClaimNonce::<T>::get(stash)
        }

        /// Debit accrued and bump claim nonce after signature verification in the precompile.
        pub fn commit_claim(
            stash: &T::AccountId,
            expected_nonce: u64,
            amount: T::RewardPoints,
        ) -> Result<(), Error<T>> {
            ensure!(
                ClaimNonce::<T>::get(stash) == expected_nonce,
                Error::<T>::BadClaimNonce
            );
            Self::take_accrued_for_claim(stash, amount)?;
            ClaimNonce::<T>::insert(stash, expected_nonce.saturating_add(1));
            Ok(())
        }

        /// Undo [`commit_claim`] if the EVM `transfer` fails (precompile only).
        pub fn undo_claim_commit(
            stash: &T::AccountId,
            nonce_before_claim: u64,
            amount: T::RewardPoints,
        ) {
            ClaimNonce::<T>::insert(stash, nonce_before_claim);
            Self::restore_accrued(stash, amount);
        }
    }
}
