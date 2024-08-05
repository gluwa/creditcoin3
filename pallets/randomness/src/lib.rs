#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

// #[cfg(test)]
// mod mock;

// #[cfg(test)]
// mod tests;

// #[cfg(feature = "runtime-benchmarks")]
// mod benchmarking;
pub mod weights;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    pub use attestor_primitives::ChainId;
    use frame_support::pallet_prelude::*;
    use frame_support::pallet_prelude::{OptionQuery, StorageMap};
    use frame_support::traits::{BuildGenesisConfig, Hooks};
    use frame_support::Blake2_128Concat;
    use frame_system::pallet_prelude::*;
    use randomness_primitives::provider::RandomnessPalletProvider;
    use sp_std::vec::Vec;

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
        fn on_initialize() -> Weight;
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

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T> {
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {}
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Initialization
        fn on_initialize(_now: BlockNumberFor<T>) -> Weight {
            let last_seen_epoch_index = LastSeenEpochIndex::<T>::get();
            let epoch_index = pallet_babe::EpochIndex::<T>::get();

            if epoch_index > last_seen_epoch_index {
                LastSeenEpochIndex::<T>::put(epoch_index);
                let randomness = pallet_babe::Randomness::<T>::get();
                RandomnessByEpochIndex::<T>::insert(epoch_index, randomness);
                Self::deposit_event(Event::StoreRandomnessForEpoch {
                    epoch_index,
                    randomness,
                });
            }

            Weight::zero()
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        StoreRandomnessForEpoch {
            epoch_index: u64,
            randomness: Randomness,
        },
    }

    #[pallet::error]
    pub enum Error<T> {}

    #[pallet::call]
    impl<T: Config> Pallet<T> {}

    impl<T: Config> RandomnessPalletProvider for Pallet<T> {
        fn randomness_by_epoch_id(epoch_id: u64) -> Option<[u8; 32]> {
            RandomnessByEpochIndex::<T>::get(epoch_id)
        }
    }
}
