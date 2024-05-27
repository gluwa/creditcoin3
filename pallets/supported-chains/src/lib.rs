#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    pub use attestor_primitives::ChainId;
    use frame_support::pallet_prelude::*;
    use frame_support::pallet_prelude::{DispatchResult, OptionQuery, StorageMap};
    use frame_support::traits::BuildGenesisConfig;
    use frame_support::Blake2_128Concat;
    use frame_system::pallet_prelude::*;
    use scale_info::prelude::string::String;
    use sp_std::vec::Vec;
    use supported_chains_primitives::provider::SupportedChainsProvider;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// A type representing the weights required by the dispatchables of this pallet.
        type WeightInfo: WeightInfo;
    }

    pub trait WeightInfo {
        fn register_chain() -> Weight;
        fn remove_chain() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn list_supported_chains)]
    pub type SupportedChains<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = ChainId,
        Value = Vec<u8>,
        QueryKind = OptionQuery,
    >;

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T> {
        pub supported_chains: Vec<(ChainId, Vec<u8>)>,
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for (chain_id, chain_name) in &self.supported_chains {
                SupportedChains::<T>::insert(chain_id, chain_name);
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        ///  A chain has been registered with a given ID
        ChainRegistered(ChainId),

        /// A chain has been removed with a given ID
        ChainRemoved(ChainId),
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The chain is already registered
        ChainAlreadyRegistered,

        /// The chain is not supported
        ChainNotSupported,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::register_chain())]
        pub fn register_chain(
            origin: OriginFor<T>,
            chain_id: ChainId,
            chain_name: String,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                !SupportedChains::<T>::contains_key(chain_id),
                Error::<T>::ChainAlreadyRegistered
            );

            SupportedChains::<T>::insert(chain_id, chain_name.as_bytes());

            Self::deposit_event(Event::ChainRegistered(chain_id));

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_chain())]
        pub fn remove_chain(origin: OriginFor<T>, chain_id: ChainId) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                SupportedChains::<T>::contains_key(chain_id),
                Error::<T>::ChainNotSupported
            );

            SupportedChains::<T>::remove(chain_id);

            Self::deposit_event(Event::ChainRemoved(chain_id));

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn is_chain_supported(chain_id: ChainId) -> bool {
            SupportedChains::<T>::contains_key(chain_id)
        }

        pub fn supported_chains() -> Option<Vec<ChainId>> {
            let chains: Vec<ChainId> = SupportedChains::<T>::iter()
                .map(|(chain_id, _)| chain_id)
                .collect();
            match chains.is_empty() {
                true => None,
                false => Some(chains),
            }
        }
    }

    impl<T: Config> SupportedChainsProvider for Pallet<T> {
        fn is_chain_supported(chain_id: ChainId) -> bool {
            Self::is_chain_supported(chain_id)
        }

        fn supported_chains() -> Option<Vec<ChainId>> {
            Self::supported_chains()
        }
    }
}
