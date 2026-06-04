#![cfg_attr(not(feature = "std"), no_std)]

//! # Randomness pallet
//!
//! Each time babe rotates to a new epoch, this pallet records that epoch's babe randomness in
//! [`RandomnessByEpochIndex`] and notifies its [`Config::EventListeners`]. In the current runtime
//! the only listener is the attestation pallet, which uses the notification purely as an
//! epoch-boundary *trigger* (to start a fresh election and apply interval updates) and ignores the
//! randomness value itself — committee selection is deterministic today.
//!
//! The randomness value is nonetheless captured and exposed (via `RandomnessPalletApi`) on
//! purpose: it is the entropy source for the future stake-weighted committee sortition described
//! in research-book RFC-0174. This pallet is the retained, VRF-ready hook for reinstating
//! probabilistic eligibility once the attestor population is large enough to need it; the VRF
//! verification primitives themselves were removed (see git history) and would be reintroduced
//! alongside that work. Until then it functions as a per-epoch timer.

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
    use randomness_primitives::{provider::RandomnessPalletProvider, OnRandomnessUpdate};

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
        type EventListeners: OnRandomnessUpdate;
        /// The maximum number of epoch-indexed randomness entries to retain. Once the
        /// store is full, inserting a new epoch evicts the oldest (lowest epoch index)
        /// entry to make room.
        #[pallet::constant]
        type MaxRandomnessEntries: Get<u32>;
    }

    pub trait WeightInfo {
        fn on_initialize() -> Weight;
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
                LastSeenEpochIndex::<T>::put(epoch_index);
                let randomness = pallet_babe::Randomness::<T>::get();

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
            RandomnessByEpochIndex::<T>::get()
                .get(&epoch_id)
                .copied()
                .unwrap_or_default()
        }
    }
}
