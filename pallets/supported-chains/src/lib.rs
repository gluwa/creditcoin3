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
    pub use attestor_primitives::{ChainId, ChainKey};
    use frame_support::{
        pallet_prelude::*,
        traits::{BuildGenesisConfig, ConstU64},
        Blake2_128Concat,
    };
    use frame_system::pallet_prelude::*;
    use scale_info::prelude::string::String;
    use sp_std::vec::Vec;
    use supported_chains_primitives::{provider::SupportedChainsProvider, SupportedChain};

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
        Key = ChainKey,
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
        ChainKey,
        OptionQuery,
    >;

    pub const GENESIS_CHAIN_KEY: ChainKey = 1;

    #[pallet::storage]
    #[pallet::getter(fn chain_key_value)]
    pub type ChainKeyValue<T> = StorageValue<_, ChainKey, ValueQuery, ConstU64<GENESIS_CHAIN_KEY>>;

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T> {
        pub supported_chains: Vec<(ChainId, Vec<u8>)>,
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let mut chain_key = 1;
            for (chain_id, chain_name) in &self.supported_chains {
                SupportedChains::<T>::insert(
                    chain_key,
                    SupportedChain {
                        chain_id: *chain_id,
                        chain_name: chain_name.clone(),
                    },
                );
                //check that no dublicate chain name is added
                if ChainIdAndNameToUniqKey::<T>::contains_key(*chain_id, chain_name.clone()) {
                    panic!("Duplicate chain name found in genesis config. Chain ID: {:?}, Chain Name: {:?}", chain_id, chain_name);
                }
                ChainIdAndNameToUniqKey::<T>::insert(*chain_id, chain_name.clone(), chain_key);
                chain_key = chain_key
                    .checked_add(1)
                    .ok_or(Error::<T>::Arithmetic)
                    .expect("Error incrementing index");
            }
            ChainKeyValue::<T>::put(chain_key);
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        ///  A chain has been registered with a given ID
        ChainRegistered {
            chain_key: ChainKey,
            chain_id: ChainId,
            chain_name: Vec<u8>,
        },

        /// A chain has been removed with a given ID
        ChainRemoved {
            chain_key: ChainKey,
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

            let chain_key = ChainKeyValue::<T>::get();
            SupportedChains::<T>::insert(
                chain_key,
                SupportedChain {
                    chain_id,
                    chain_name: chain_name.as_bytes().to_vec(),
                },
            );
            ChainIdAndNameToUniqKey::<T>::insert(
                chain_id,
                chain_name.as_bytes().to_vec(),
                chain_key,
            );

            ChainKeyValue::<T>::put(chain_key.checked_add(1).ok_or(Error::<T>::Arithmetic)?);

            Self::deposit_event(Event::ChainRegistered {
                chain_key,
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
            });

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_chain())]
        pub fn remove_chain(origin: OriginFor<T>, chain_key: ChainId) -> DispatchResult {
            ensure_root(origin)?;

            let item = SupportedChains::<T>::get(chain_key).ok_or(Error::<T>::ChainNotSupported)?;

            ChainIdAndNameToUniqKey::<T>::remove(item.chain_id, item.chain_name.clone());

            SupportedChains::<T>::remove(chain_key);

            Self::deposit_event(Event::ChainRemoved {
                chain_key,
                chain_id: item.chain_id,
                chain_name: item.chain_name.clone(),
            });

            Ok(())
        }
    }

    impl<T: Config> SupportedChainsProvider for Pallet<T> {
        fn is_chain_supported(chain_id: ChainKey) -> bool {
            SupportedChains::<T>::contains_key(chain_id)
        }

        fn supported_chains() -> Option<Vec<ChainKey>> {
            let chains: Vec<ChainKey> = SupportedChains::<T>::iter()
                .map(|(chain_id, _)| chain_id)
                .collect();
            match chains.is_empty() {
                true => None,
                false => Some(chains),
            }
        }

        fn chain_key_by_chain_id_and_name(
            chain_id: ChainId,
            chain_name: Vec<u8>,
        ) -> Option<ChainId> {
            ChainIdAndNameToUniqKey::<T>::get(chain_id, chain_name)
        }
    }
}
