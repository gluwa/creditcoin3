#![cfg_attr(not(feature = "std"), no_std)]

pub mod types;
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
    use proof_verifier::host_api::verify_proof;
    use prover_primitives::claim::Claim;
    use sp_runtime::traits::{Hash, SaturatedConversion};
    use sp_std::{fmt::Debug, vec::Vec};
    use supported_chains_primitives::provider::SupportedChainsProvider;

    use prover_primitives::host_api::verify_proof;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        type Address: Codec + Encode + Decode + TypeInfo + Clone + Debug + Eq + PartialEq;
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
        Value = BTreeMap<T::Hash, Claim<<T as Config>::Address>>,
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

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub provers: Vec<(T::AccountId, Vec<(ChainId, ChainPriceConfiguration)>)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let provers = &self.provers;
            for (prover, chain_prices) in provers.iter() {
                Provers::<T>::insert(prover, Prover { nickname: vec![] });

                for price_config in chain_prices.iter() {
                    ProversChainPriceConfigurations::<T>::insert(
                        prover,
                        price_config.0,
                        Some(price_config.clone().1),
                    );
                }
            }
        }
    }

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

        InvalidProofSubmitted,

        BalanceToLow,
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
        ) -> DispatchResult {
            let source = ensure_signed(origin)?;

            // Take a prover from the list of provers
            let prover = Provers::<T>::iter()
                .next()
                .ok_or(Error::<T>::ProverNotExists)?;

            // Get the prover price for a claim on that chain
            let prover_chain_price =
                ProversChainPriceConfigurations::<T>::get(&prover.0, claim.chain_id)
                    .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            let balance = T::Currency::free_balance(&source);
            if balance < BalanceOf::<T>::saturated_from(prover_chain_price.price) {
                return Err(Error::<T>::BalanceToLow.into());
            };

            // Lock an amount equal to the price
            let price = LockedBalanceOf::<T>::saturated_from(prover_chain_price.price);
            T::ClaimLockCurrency::set_lock(LOCK_ID, &source, price, WithdrawReasons::all());

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

            // Pre eliminary check
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            // Verify proof
            // Need to find a way to generate a proof in test mode and verify it
            // Currently not really implemented
            ensure!(
                verify_proof(proof.clone()),
                Error::<T>::InvalidProofSubmitted
            );

            let prover_claims = ProverClaims::<T>::get(&prover);
            let claim = prover_claims
                .get(&claim_hash)
                .ok_or(Error::<T>::ClaimNotExists)?;

            // Check hash
            let claim_h = <T as Config>::Hashing::hash_of(&claim);
            ensure!(claim_hash == claim_h, Error::<T>::WrongClaimHash);

            // Remove lock
            // TODO: remove only partially since source can have many claims at the same time
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
                &claim_source,
                &prover,
                price,
                ExistenceRequirement::KeepAlive,
            )?;

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {}
}
