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
    use supported_chains_primitives::SupportedChain;

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
        Value = SupportedChain,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn chain_id_and_name_to_uniq_key)]
    pub type ChainIdAndNameToUniqKey<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        ChainId,
        Blake2_128Concat,
        Vec<u8>,
        ChainId,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn incremental_key)]
    pub type IncrementalKey<T> = StorageValue<_, ChainId, ValueQuery>;

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T> {
        pub supported_chains: Vec<(ChainId, Vec<u8>)>,
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let mut index = 0;
            for (chain_id, chain_name) in &self.supported_chains {
                SupportedChains::<T>::insert(
                    *chain_id,
                    SupportedChain {
                        chain_id: *chain_id,
                        chain_name: chain_name.clone(),
                    },
                );
                //check that no dublicate chain name is added
                if ChainIdAndNameToUniqKey::<T>::contains_key(*chain_id, chain_name.clone()) {
                    panic!("Duplicate chain name found in genesis config. Chain ID: {:?}, Chain Name: {:?}", chain_id, chain_name);
                }
                ChainIdAndNameToUniqKey::<T>::insert(*chain_id, chain_name.clone(), index);
                index = index
                    .checked_add(1)
                    .ok_or(Error::<T>::Arithmetic)
                    .expect("Error incrementing index");
            }
            IncrementalKey::<T>::put(index);
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        ///  A chain has been registered with a given ID
        ChainRegistered {
            generated_key: ChainId,
            chain_id: ChainId,
            chain_name: Vec<u8>,
        },

        /// A chain has been removed with a given ID
        ChainRemoved {
            generated_key: ChainId,
            chain_id: ChainId,
            chain_name: Vec<u8>,
        },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The chain is already registered
        ChainAlreadyRegistered,

        /// The chain is not supported
        ChainNotSupported,

        /// Math overflow/underflow
        Arithmetic,
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
                !ChainIdAndNameToUniqKey::<T>::contains_key(chain_id, chain_name.as_bytes()),
                Error::<T>::ChainAlreadyRegistered
            );

            let incremental_key = IncrementalKey::<T>::get();
            SupportedChains::<T>::insert(
                chain_id,
                SupportedChain {
                    chain_id,
                    chain_name: chain_name.as_bytes().to_vec(),
                },
            );
            ChainIdAndNameToUniqKey::<T>::insert(
                chain_id,
                chain_name.as_bytes().to_vec(),
                incremental_key,
            );

            IncrementalKey::<T>::put(
                incremental_key
                    .checked_add(1)
                    .ok_or(Error::<T>::Arithmetic)?,
            );

            Self::deposit_event(Event::ChainRegistered {
                generated_key: incremental_key,
                chain_id: chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
            });

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_chain())]
        pub fn remove_chain(origin: OriginFor<T>, generated_key: ChainId) -> DispatchResult {
            ensure_root(origin)?;

            let item =
                SupportedChains::<T>::get(generated_key).ok_or(Error::<T>::ChainNotSupported)?;

            ChainIdAndNameToUniqKey::<T>::remove(item.chain_id, item.chain_name.clone());

            SupportedChains::<T>::remove(generated_key);

            Self::deposit_event(Event::ChainRemoved {
                generated_key,
                chain_id: item.chain_id,
                chain_name: item.chain_name.clone(),
            });

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn is_chain_supported(generated_key: ChainId) -> bool {
            SupportedChains::<T>::contains_key(generated_key)
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
