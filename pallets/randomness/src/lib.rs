#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
use randomness_primitives::Randomness;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    pub use attestor_primitives::ChainId;
    use frame_support::pallet_prelude::StorageMap;
    use frame_support::pallet_prelude::*;
    use frame_support::Blake2_128Concat;
    use frame_system::pallet_prelude::*;
    use randomness_primitives::{provider::RandomnessPalletProvider, OnRandomnessUpdate};

    pub const RANDOMNESS_LENGTH: usize = 32;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_babe::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// A type representing the weights required by the dispatchables of this pallet.
        type WeightInfo: WeightInfo;
        /// Something that notifies the pallet about randomness updates.
        type EventListeners: OnRandomnessUpdate;
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
        QueryKind = ValueQuery,
    >;

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

                // Notify event listeners
                T::EventListeners::on_new_epoch_randomness(epoch_index, randomness);

                Self::deposit_event(Event::StoreRandomnessForEpoch {
                    epoch_index,
                    randomness,
                });
            }

            <T as pallet::Config>::WeightInfo::on_initialize()
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
        fn randomness_by_epoch_id(epoch_id: u64) -> Randomness {
            RandomnessByEpochIndex::<T>::get(epoch_id)
        }
    }
}
