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
    use frame_support::pallet_prelude::{
        CountedStorageMap, DispatchResult, OptionQuery, StorageMap,
    };
    use frame_support::{pallet_prelude::*, Blake2_128Concat};
    use frame_system::pallet_prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
    }

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
        ProverChainPriceConfigurationSet(T::AccountId, ChainPriceConfiguration),
        ProverChainPriceConfigurationUnset(T::AccountId, ChainPriceConfiguration),
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Prover already registered
        ProverAlreadyRegistered,

        /// Prover not exists
        ProverNotExists,

        ProverAlreadyExists,

        ChainPriceConfigurationNotFound,
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
            chain_price_config: ChainPriceConfiguration,
        ) -> DispatchResult {
            let address = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&address),
                Error::<T>::ProverNotExists
            );

            // TODO: check if we can actually set a price for this chainID

            let mut chain_configs =
                ProversChainPriceConfigurations::<T>::get(&address).unwrap_or_default();
            chain_configs.push(chain_price_config.clone());

            ProversChainPriceConfigurations::<T>::insert(&address, &chain_configs);

            Self::deposit_event(Event::<T>::ProverChainPriceConfigurationSet(
                address,
                chain_price_config,
            ));

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::unset_chain_price_config())]
        pub fn unset_chain_price_config(
            origin: OriginFor<T>,
            chain_price_config: ChainPriceConfiguration,
        ) -> DispatchResult {
            let address = ensure_signed(origin)?;

            ensure!(
                Provers::<T>::contains_key(&address),
                Error::<T>::ProverNotExists
            );

            let mut chain_configs = ProversChainPriceConfigurations::<T>::get(&address)
                .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            // Find index and remove from vec
            let index = chain_configs
                .iter()
                .position(|cfg| cfg == &chain_price_config)
                .ok_or(Error::<T>::ChainPriceConfigurationNotFound)?;

            chain_configs.remove(index);

            ProversChainPriceConfigurations::<T>::insert(&address, &chain_configs);

            Self::deposit_event(Event::<T>::ProverChainPriceConfigurationUnset(
                address,
                chain_price_config,
            ));

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {}
}
