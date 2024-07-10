#![cfg_attr(not(feature = "std"), no_std)]

pub mod types;
pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{Claim, Prover};

    use attestor_primitives::ChainId;
    use frame_support::{
        dispatch::DispatchResult,
        pallet_prelude::{ValueQuery, *},
        traits::{
            Currency, ExistenceRequirement, LockIdentifier, LockableCurrency, WithdrawReasons,
        },
        Blake2_128Concat,
    };
    use frame_system::pallet_prelude::{BlockNumberFor, *};
    use parity_scale_codec::Codec;
    use proof_verifier::host_api::verify_proof;
    pub use prover_primitives::ChainPriceConfiguration;
    use sp_runtime::traits::{CheckedAdd, CheckedSub, Hash, SaturatedConversion, Zero};
    use sp_std::vec::Vec;
    use sp_std::{collections::btree_map::BTreeMap, fmt::Debug, vec};
    use supported_chains_primitives::provider::SupportedChainsProvider;

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        type Address: Codec + Encode + Decode + TypeInfo + Clone + Debug + Eq + PartialEq + Default;
        type Currency: Currency<Self::AccountId>;
        type ClaimLockCurrency: LockableCurrency<Self::AccountId, Moment = BlockNumberFor<Self>>;
        type Hashing: Hash<Output = Self::Hash>;
        type SupportedChains: SupportedChainsProvider;
    }

    const LOCK_ID: LockIdentifier = *b"claimlck";

    type LockedBalanceOf<T> = <<T as Config>::ClaimLockCurrency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    type BalanceOf<T> =
        <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

    pub trait WeightInfo {
        fn register_prover() -> Weight;
        fn set_chain_price_config() -> Weight;
        fn unset_chain_price_config() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn provers)]
    pub type Provers<T: Config> = CountedStorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        Value = Prover,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn provers_chain_price_configs)]
    pub type ProversChainPriceConfigurations<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        Value = Vec<ChainPriceConfiguration>,
        QueryKind = ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn prover_claims)]
    pub type ProverClaims<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        Value = BTreeMap<T::Hash, Claim>,
        QueryKind = ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn claim_source_by_hash)]
    pub type ClaimSourceByHash<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = T::Hash,
        Value = T::AccountId,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn claim_result_by_hash)]
    pub type ClaimResultByHash<T: Config> =
        StorageMap<Hasher = Blake2_128Concat, Key = T::Hash, Value = bool, QueryKind = OptionQuery>;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub provers: Vec<(T::AccountId, Vec<ChainPriceConfiguration>)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let provers = &self.provers;
            for (prover, chain_prices) in provers.iter() {
                Provers::<T>::insert(prover, Prover { nickname: vec![] });
                ProversChainPriceConfigurations::<T>::insert(prover, chain_prices);
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Emitted when an prover is properly registered with the prover system
        ProverRegistered(T::AccountId),

        ///
        ProverChainPriceConfigurationSet(T::AccountId, Vec<ChainPriceConfiguration>),

        ///
        ProverClaimSubmitted(T::AccountId, T::AccountId, T::Hash, Claim),

        ///
        ClaimVerified(T::Hash, T::AccountId),
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Prover already registered
        ProverAlreadyRegistered,

        /// Prover not exists
        ProverNotExists,

        ProverAlreadyExists,

        ChainPriceConfigurationNotFound,

        ClaimNotExists,

        WrongClaimHash,

        InvalidProofSubmitted,

        BalanceToLow,

        ChainNotSupported,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::register_prover())]
        pub fn register_prover(origin: OriginFor<T>, prover: Prover) -> DispatchResult {
            let address = ensure_signed(origin)?;

            ensure!(
                !Provers::<T>::contains_key(&address),
                Error::<T>::ProverAlreadyExists
            );

            Provers::<T>::insert(&address, prover);

            Self::deposit_event(Event::<T>::ProverRegistered(address));

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::set_chain_price_config())]
        pub fn set_chain_price_config(
            origin: OriginFor<T>,
            chain_price_configs: Vec<ChainPriceConfiguration>,
        ) -> DispatchResult {
            let address = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&address),
                Error::<T>::ProverNotExists
            );

            // Validate the chain ids
            for chain_price_config in chain_price_configs.iter() {
                ensure!(
                    T::SupportedChains::is_chain_supported(chain_price_config.chain_id),
                    Error::<T>::ChainNotSupported
                );
            }

            ProversChainPriceConfigurations::<T>::insert(&address, &chain_price_configs);

            Self::deposit_event(Event::<T>::ProverChainPriceConfigurationSet(
                address,
                chain_price_configs,
            ));

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::unset_chain_price_config())]
        pub fn submit_claim(origin: OriginFor<T>, claim: Claim) -> DispatchResult {
            let source = ensure_signed(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(claim.chain_id),
                Error::<T>::ChainNotSupported
            );

            // Take a prover from the list of provers
            let prover = Provers::<T>::iter()
                .next()
                .ok_or(Error::<T>::ProverNotExists)?;

            // Lock the funds for this claim
            Self::lock_for_claim(&source, &claim, &prover.0)?;

            // Hash the claim
            let claim_hash = <T as Config>::Hashing::hash_of(&claim);

            // Insert the claim
            let mut prover_claims = ProverClaims::<T>::get(&prover.0);
            prover_claims.insert(claim_hash, claim.clone());
            ProverClaims::<T>::insert(&prover.0, prover_claims);

            // Insert claim source by hash
            ClaimSourceByHash::<T>::insert(claim_hash, &source);

            Self::deposit_event(Event::<T>::ProverClaimSubmitted(
                source, prover.0, claim_hash, claim,
            ));

            Ok(())
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unset_chain_price_config())]
        pub fn submit_proof(
            origin: OriginFor<T>,
            claim_hash: T::Hash,
            proof: Vec<u8>,
        ) -> DispatchResult {
            let prover = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&prover),
                Error::<T>::ProverNotExists
            );

            // Pre eliminary check
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            // Verify proof
            // Need to find a way to generate a proof in test mode and verify it
            // Currently not really implemented
            
            #[cfg(not(feature = "runtime-benchmarks"))]
            ensure!(verify_proof(proof), Error::<T>::InvalidProofSubmitted);

            // Check existence
            let prover_claims = ProverClaims::<T>::get(&prover);
            let claim = prover_claims
                .get(&claim_hash)
                .ok_or(Error::<T>::ClaimNotExists)?;

            // Check hash
            let claim_h = <T as Config>::Hashing::hash_of(&claim);
            ensure!(claim_hash == claim_h, Error::<T>::WrongClaimHash);

            // Unlock funds
            Self::unlock_for_claim(&prover, claim, &claim_hash)?;

            // Store result
            ClaimResultByHash::<T>::insert(claim_hash, true);

            // Deposit event
            Self::deposit_event(Event::<T>::ClaimVerified(claim_hash, prover));

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn lock_for_claim(
            source: &T::AccountId,
            claim: &Claim,
            prover: &T::AccountId,
        ) -> DispatchResult {
            // Get the prover price for a claim on that chain
            let prover_chain_price = Self::prover_chain_price(prover, claim.chain_id)
                .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            let balance = T::Currency::free_balance(source);
            if balance < BalanceOf::<T>::saturated_from(prover_chain_price.price) {
                return Err(Error::<T>::BalanceToLow.into());
            };

            let price = BalanceOf::<T>::saturated_from(prover_chain_price.price);

            // Get the locked balance
            let mut locked_balance = Self::get_locked_balance(source);

            // Update the locked balance
            // Locked balance + the amount to pay for computing this claim
            locked_balance = locked_balance
                .checked_add(&price)
                .unwrap_or(BalanceOf::<T>::zero());

            // Extend (or set) the lock
            <T as Config>::ClaimLockCurrency::extend_lock(
                LOCK_ID,
                source,
                LockedBalanceOf::<T>::saturated_from(locked_balance.saturated_into::<u128>()),
                WithdrawReasons::RESERVE,
            );

            Ok(())
        }

        pub fn unlock_for_claim(
            prover: &T::AccountId,
            claim: &Claim,
            claim_hash: &T::Hash,
        ) -> DispatchResult {
            let claim_source =
                ClaimSourceByHash::<T>::get(claim_hash).ok_or(Error::<T>::ClaimNotExists)?;

            // Get the prover price for a claim on that chain
            let prover_chain_price = Self::prover_chain_price(prover, claim.chain_id)
                .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            // Get locked balance
            let locked_balance = Self::get_locked_balance(&claim_source);

            // Calculate newly locked balance
            // Locked balance - the price for the claim
            let price = BalanceOf::<T>::saturated_from(prover_chain_price.price);
            let new_locked_balance = locked_balance
                .checked_sub(&price)
                .unwrap_or(BalanceOf::<T>::zero());

            // Set the newly locked balance
            <T as Config>::ClaimLockCurrency::set_lock(
                LOCK_ID,
                &claim_source,
                LockedBalanceOf::<T>::saturated_from(new_locked_balance.saturated_into::<u128>()),
                WithdrawReasons::RESERVE,
            );

            // Now transfer
            T::Currency::transfer(
                &claim_source,
                prover,
                price,
                ExistenceRequirement::KeepAlive,
            )?;

            Ok(())
        }

        // Get the usable balance of an account
        // This is the balance minus the minimum balance
        pub fn get_locked_balance(account_id: &T::AccountId) -> BalanceOf<T> {
            let balance_locks = pallet_balances::pallet::Pallet::<T>::locks(account_id);

            let mut locked_balance = BalanceOf::<T>::zero();
            // loop over balance and check if the identifier matches the one we have, if so, accumulate
            for lock in balance_locks {
                if lock.id == LOCK_ID {
                    locked_balance +=
                        BalanceOf::<T>::saturated_from(lock.amount.saturated_into::<u128>());
                }
            }

            locked_balance
        }

        pub fn prover_chain_price(
            prover: &T::AccountId,
            chain_id: ChainId,
        ) -> Option<ChainPriceConfiguration> {
            let chain_prices = ProversChainPriceConfigurations::<T>::get(prover);

            chain_prices.into_iter().find(|c| c.chain_id == chain_id)
        }
    }
}
