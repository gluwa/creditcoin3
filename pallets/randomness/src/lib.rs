#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod migrations;
pub mod weights;
use randomness_primitives::Randomness;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::pallet_prelude::*;
    use frame_support::BoundedBTreeMap;
    use frame_system::pallet_prelude::*;
    use randomness_primitives::{
        provider::RandomnessPalletProvider, OnRandomnessUpdate, OnRandomnessUpdateWeight,
    };

    pub const RANDOMNESS_LENGTH: usize = 32;

    /// The in-code storage version.
    const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_babe::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// A type representing the weights required by the dispatchables of this pallet.
        type WeightInfo: WeightInfo;
        /// Something that notifies the pallet about randomness updates.
        type EventListeners: OnRandomnessUpdate + OnRandomnessUpdateWeight;
        /// The maximum number of epoch-indexed randomness entries to retain. Once the
        /// store is full, inserting a new epoch evicts the oldest (lowest epoch index)
        /// entry to make room.
        #[pallet::constant]
        type MaxRandomnessEntries: Get<u32>;
    }

    pub trait WeightInfo {
        /// Weight of `on_initialize` when the epoch has not advanced (the common
        /// case): only the two storage reads used to detect a new epoch.
        fn on_initialize() -> Weight;
        /// Weight of `on_initialize` on an epoch boundary: the reads above plus
        /// writing `LastSeenEpochIndex`, inserting into the bounded randomness
        /// map (with a possible eviction), and depositing the event. The listener
        /// cost is added separately via [`OnRandomnessUpdateWeight`].
        fn on_initialize_epoch_change() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn epoch_index)]
    pub type LastSeenEpochIndex<T> = StorageValue<_, u64, ValueQuery>;

    /// Randomness keyed by epoch index, bounded to the latest
    /// [`Config::MaxRandomnessEntries`] epochs. Backed by a [`BoundedBTreeMap`] so
    /// entries stay ordered by epoch index, which makes evicting the oldest entry
    /// (the lowest key) cheap.
    #[pallet::storage]
    #[pallet::getter(fn randomness_by_epoch_index)]
    pub type RandomnessByEpochIndex<T: Config> =
        StorageValue<_, BoundedBTreeMap<u64, Randomness, T::MaxRandomnessEntries>, ValueQuery>;

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Initialization
        fn on_initialize(_now: BlockNumberFor<T>) -> Weight {
            let last_seen_epoch_index = LastSeenEpochIndex::<T>::get();
            let epoch_index = pallet_babe::EpochIndex::<T>::get();

            if epoch_index > last_seen_epoch_index {
                let randomness = pallet_babe::Randomness::<T>::get();

                // Pallet-local store + event work (no listener).
                Self::store_epoch_randomness(epoch_index, randomness);

                // Notify event listeners (in the runtime: attestor election +
                // interval updates).
                T::EventListeners::on_new_epoch_randomness(epoch_index, randomness);

                // Epoch boundary weight, assembled from three independently
                // benchmarked pieces so nothing is double- or under-counted:
                //
                // * `on_initialize` covers the new-epoch detection reads
                //   (`LastSeenEpochIndex` + `Babe::EpochIndex`) that run on every
                //   block before the branch is taken, plus one extra read for
                //   `Babe::Randomness` which is only loaded on the epoch branch.
                // * `on_initialize_epoch_change` covers this pallet's own
                //   store/event work, benchmarked via `store_epoch_randomness`
                //   so it deliberately excludes the listener.
                // * `on_new_epoch_randomness_weight` is the listener's own
                //   benchmarked weight reported through `OnRandomnessUpdateWeight`
                //   (its real election/interval cost rather than being measured
                //   against an empty attestor set).
                return <T as pallet::Config>::WeightInfo::on_initialize()
                    .saturating_add(T::DbWeight::get().reads(1))
                    .saturating_add(<T as pallet::Config>::WeightInfo::on_initialize_epoch_change())
                    .saturating_add(T::EventListeners::on_new_epoch_randomness_weight());
            }

            // No epoch change: only the two detection reads ran.
            <T as pallet::Config>::WeightInfo::on_initialize()
        }
    }

    impl<T: Config> Pallet<T> {
        /// Store `randomness` for `epoch_index`, evicting the oldest entry when the
        /// bounded map is full, bump `LastSeenEpochIndex`, and emit the event.
        ///
        /// This is the pallet-local epoch-change work, factored out so the
        /// `on_initialize_epoch_change` benchmark can measure it *without* running
        /// the configured `EventListeners` (whose cost is accounted separately via
        /// [`OnRandomnessUpdateWeight`]).
        pub(crate) fn store_epoch_randomness(epoch_index: u64, randomness: Randomness) {
            LastSeenEpochIndex::<T>::put(epoch_index);

            RandomnessByEpochIndex::<T>::mutate(|map| {
                // If the store is full, evict the oldest entry (the lowest epoch
                // index, which is the first key in the ordered map) to make room.
                if map.len() >= T::MaxRandomnessEntries::get() as usize {
                    if let Some(oldest) = map.keys().next().copied() {
                        map.remove(&oldest);
                    }
                }

                // We just ensured there is room, so this insert cannot fail. If it
                // somehow does (e.g. a zero bound), there is nothing sensible to do
                // beyond skipping it.
                let _ = map.try_insert(epoch_index, randomness);
            });

            Self::deposit_event(Event::StoreRandomnessForEpoch {
                epoch_index,
                randomness,
            });
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
            RandomnessByEpochIndex::<T>::get()
                .get(&epoch_id)
                .copied()
                .unwrap_or_default()
        }
    }
}
