#![cfg_attr(not(feature = "std"), no_std)]

mod types;
pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{ChainPriceConfiguration, Prover};
    use attestor_primitives::ChainId;
    use frame_support::{
        pallet_prelude::{ValueQuery, *},
        traits::{
            Currency, ExistenceRequirement, LockIdentifier, LockableCurrency, WithdrawReasons,
        },
        Blake2_128Concat,
    };
    use frame_system::pallet_prelude::{BlockNumberFor, *};
    use parity_scale_codec::Codec;
    use prover_primitives::claim::Claim;
    use sp_runtime::traits::{Hash, SaturatedConversion};
    use sp_std::fmt::Debug;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        type Address: Codec + Encode + Decode + TypeInfo + Clone + Debug + Eq + PartialEq;
        type Currency: Currency<Self::AccountId>;
        type ClaimLockCurrency: LockableCurrency<Self::AccountId, Moment = BlockNumberFor<Self>>;
        type Hashing: Hash<Output = Self::Hash>;
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
    pub type ProversChainPriceConfigurations<T: Config> = StorageDoubleMap<
        Hasher1 = Blake2_128Concat,
        Key1 = T::AccountId,
        Hasher2 = Blake2_128Concat,
        Key2 = ChainId,
        Value = Option<ChainPriceConfiguration>,
        QueryKind = ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn prover_claims)]
    pub type ProverClaims<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        Value = Claim<<T as Config>::Address>,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn claim_source_by_hash)]
    pub type ClaimSourceByHash<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = T::Hash,
        Value = T::AccountId,
        QueryKind = OptionQuery,
    >;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Emitted when an prover is properly registered with the prover system
        ProverRegistered(T::AccountId),

        ///
        ProverChainPriceConfigurationSet(T::AccountId, ChainId, Option<ChainPriceConfiguration>),

        ///
        ProverClaimSubmitted(
            T::AccountId,
            T::AccountId,
            T::Hash,
            Claim<<T as Config>::Address>,
        ),
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

        ClaimInProgress,

        InvalidProofSubmitted,
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
            chain_id: ChainId,
            chain_price_config: Option<ChainPriceConfiguration>,
        ) -> DispatchResult {
            let address = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&address),
                Error::<T>::ProverNotExists
            );

            ProversChainPriceConfigurations::<T>::insert(&address, chain_id, &chain_price_config);

            Self::deposit_event(Event::<T>::ProverChainPriceConfigurationSet(
                address,
                chain_id,
                chain_price_config,
            ));

            Ok(())
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unset_chain_price_config())]
        pub fn submit_claim(
            origin: OriginFor<T>,
            claim: Claim<<T as Config>::Address>,
            prover: T::AccountId,
        ) -> DispatchResult {
            let source = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&prover),
                Error::<T>::ProverNotExists
            );

            ensure!(
                !ProverClaims::<T>::contains_key(&prover),
                Error::<T>::ClaimInProgress
            );

            // Get the prover price for a claim on that chain
            let prover_chain_price =
                ProversChainPriceConfigurations::<T>::get(&prover, claim.chain_id)
                    .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            // Lock an amount equal to the price
            let price = LockedBalanceOf::<T>::saturated_from(prover_chain_price.price);
            T::ClaimLockCurrency::set_lock(LOCK_ID, &source, price, WithdrawReasons::all());

            // Insert the claim
            ProverClaims::<T>::insert(&prover, &claim);

            let claim_hash = <T as Config>::Hashing::hash_of(&claim);

            ClaimSourceByHash::<T>::insert(claim_hash, &source);

            Self::deposit_event(Event::<T>::ProverClaimSubmitted(
                source, prover, claim_hash, claim,
            ));

            Ok(())
        }

        #[pallet::call_index(4)]
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

            // TODO: How to even validate if this proof is good?
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            let claim = ProverClaims::<T>::get(&prover).ok_or(Error::<T>::ClaimNotExists)?;

            // Check hash
            let claim_h = <T as Config>::Hashing::hash_of(&claim);
            ensure!(claim_hash == claim_h, Error::<T>::WrongClaimHash);

            // Remove lock
            let claim_source =
                ClaimSourceByHash::<T>::get(claim_hash).ok_or(Error::<T>::ClaimNotExists)?;
            T::ClaimLockCurrency::remove_lock(LOCK_ID, &claim_source);

            // Get the prover price for a claim on that chain
            let prover_chain_price =
                ProversChainPriceConfigurations::<T>::get(&prover, claim.chain_id)
                    .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            // Transfer this amount to the prover from the source
            let price = BalanceOf::<T>::saturated_from(prover_chain_price.price);
            T::Currency::transfer(
                &prover,
                &claim_source,
                price,
                ExistenceRequirement::KeepAlive,
            )?;

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {}
}
