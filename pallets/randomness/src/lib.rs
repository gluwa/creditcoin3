#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;

use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use randomness_primitives::Randomness;
use scale_info::TypeInfo;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PruningState {
    // The next epoch index to prune from.
    pub next: u64,
    // The epoch index to prune to (inclusive).
    pub to: u64,
}

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::pallet_prelude::StorageMap;
    use frame_support::pallet_prelude::*;
    use frame_support::Blake2_128Concat;
    use frame_system::pallet_prelude::*;
    use randomness_primitives::{provider::RandomnessPalletProvider, OnRandomnessUpdate};

    pub const RANDOMNESS_LENGTH: usize = 32;

    /// The in-code storage version.
    const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_babe::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// A type representing the weights required by the dispatchables of this pallet.
        type WeightInfo: WeightInfo;
        /// Something that notifies the pallet about randomness updates.
        type EventListeners: OnRandomnessUpdate;
        /// Number of past epochs for which randomness is retained in storage.
        /// Entries older than this are pruned each time a new epoch is recorded.
        #[pallet::constant]
        type MaxEpochHistory: Get<u64>;
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

    #[pallet::storage]
    pub type PruningQueue<T: Config> = StorageValue<_, PruningState, OptionQuery>;

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

                let max_epoch_history = T::MaxEpochHistory::get();
                let prune_to = epoch_index.checked_sub(max_epoch_history);
                let prune_from = last_seen_epoch_index
                    .checked_sub(max_epoch_history)
                    .map_or(prune_to, Some);

                // If the new epoch index exceeds the max history, we need to update the pruning queue.
                if let (Some(from), Some(to)) = (prune_from, prune_to) {
                    PruningQueue::<T>::mutate(|maybe_prunning_state| match maybe_prunning_state {
                        Some(ref mut prunning_state) => {
                            // Extending `to` is sufficient: the current `next` is always <= the new `from`,
                            // so the queue will sweep through the gap as it advances each block.
                            prunning_state.to = prunning_state.to.max(to);
                        }
                        None => {
                            *maybe_prunning_state = Some(PruningState { next: from, to });
                        }
                    });
                }

                // Notify event listeners
                T::EventListeners::on_new_epoch_randomness(epoch_index, randomness);

                Self::deposit_event(Event::StoreRandomnessForEpoch {
                    epoch_index,
                    randomness,
                });
            }

            // We try to prune one entry from the queue each block, to avoid a potential weight spike from pruning many entries at once.
            PruningQueue::<T>::mutate(|maybe_prunning_state| {
                if let Some(ref mut prunning_state) = maybe_prunning_state {
                    let epoch_to_prune = prunning_state.next;
                    RandomnessByEpochIndex::<T>::remove(epoch_to_prune);
                    prunning_state.next = prunning_state.next.saturating_add(1);

                    // If we've pruned up to the target epoch, clear the prunning state.
                    if prunning_state.next > prunning_state.to {
                        *maybe_prunning_state = None;
                    }
                }
            });

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
