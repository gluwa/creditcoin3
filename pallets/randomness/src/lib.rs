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
    use frame_support::traits::{BuildGenesisConfig, Hooks};
    use frame_support::Blake2_128Concat;
    use frame_system::pallet_prelude::*;
    use scale_info::prelude::string::String;
    use sp_std::vec::Vec;
    use supported_chains_primitives::provider::SupportedChainsProvider;

    pub const RANDOMNESS_LENGTH: usize = 32;

/// Randomness type required by BABE operations.
    pub type Randomness = [u8; RANDOMNESS_LENGTH];

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_babe::Config {
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
	#[pallet::getter(fn epoch_index)]
	pub type LastSeenEpochIndex<T> = StorageValue<_, u64, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn randomness_by_epoch_index)]
    pub type RandomnessByEpochIndex<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = u64,
        Value = Randomness,
        QueryKind = OptionQuery,
    >;

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

    #[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/// Initialization
		fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let last_seen_epoch_index = LastSeenEpochIndex::<T>::get();
            let epoch_index = pallet_babe::EpochIndex::<T>::get();
            if epoch_index > last_seen_epoch_index {
                LastSeenEpochIndex::<T>::put(epoch_index);
                let randomness = pallet_babe::Randomness::<T>::get();
                RandomnessByEpochIndex::<T>::insert(epoch_index, randomness);
                Self::deposit_event(Event::StoreRandomnessForEpoch{ epoch_index, randomness });
            }
            
			Weight::zero()
		}
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        StoreRandomnessForEpoch{
            epoch_index: u64, 
            randomness: Randomness
        }
    }

    #[pallet::error]
    pub enum Error<T> {
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
       
    }
}
