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
    use attestor_primitives::{ChainEncodingVersion, ChainKey};
    use frame_support::{
        pallet_prelude::*,
        traits::{BuildGenesisConfig, ConstU64},
        Blake2_128Concat,
    };
    use frame_system::pallet_prelude::*;
    use scale_info::prelude::string::String;
    use sp_std::vec::Vec;
    use supported_chains_primitives::{
        chain_removal_listener::ChainRemovalListener,
        provider::{OnRegisterChainProvider as OnChainRegisteredProvider, SupportedChainsProvider},
        SupportedChain,
    };

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// A type representing the weights required by the dispatchables of this pallet.
        type WeightInfo: WeightInfo;
        /// Informs other pallets if a supported chain is removed
        type EventListeners: ChainRemovalListener;
        type ChainRegistrationHandler: OnChainRegisteredProvider;
        /// Determines which maturity strategy attestors will use when fetching source chain
        /// blocks and building attestations. Strict strategies such as EvmFinalized delay
        /// longer before building attestations, ensuring that attestations won't be created
        /// from blocks which could be re-orged.
        ///
        /// Maturity strategies use the String type for extensability.
        ///
        /// Expected strategies:
        /// - "EvmFinalized" Gets blocks once they are finalized
        /// - "EvmSafe" Gets blocks once they are confirmed
        /// - "EvmLatest" Gets blocks as soon as available
        /// - "FixedDelay: X" Gets blocks after they are X blocks old
        #[pallet::constant]
        type DefaultMaturityStrategy: Get<String>;
    }

    pub trait WeightInfo {
        fn register_chain() -> Weight;
        fn remove_chain() -> Weight;
        fn set_maturity_strategy() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn supported_chain)]
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
        pub supported_chains: Vec<(ChainId, Vec<u8>, ChainEncodingVersion, String)>,
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let mut chain_key = 1;
            for (chain_id, chain_name, encoding, maturity_strategy) in &self.supported_chains {
                SupportedChains::<T>::insert(
                    chain_key,
                    SupportedChain {
                        chain_id: *chain_id,
                        chain_name: chain_name.clone(),
                        chain_encoding: *encoding,
                        maturity_strategy: maturity_strategy.clone(),
                    },
                );
                //check that no dublicate chain name is added
                if ChainIdAndNameToUniqKey::<T>::contains_key(*chain_id, chain_name.clone()) {
                    panic!("Duplicate chain name found in genesis config. Chain ID: {chain_id:?}, Chain Name: {chain_name:?}");
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
            chain_encoding: ChainEncodingVersion,
        },

        /// A chain has been removed with a given ID
        ChainRemoved {
            chain_key: ChainKey,
            chain_id: ChainId,
            chain_name: Vec<u8>,
            chain_encoding: ChainEncodingVersion,
        },

        /// The maturity strategy for a chain has been set
        MaturityStrategySet {
            chain_key: ChainKey,
            chain_id: ChainId,
            chain_name: Vec<u8>,
            maturity_strategy: String,
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

        /// Maturity strategy doesn't match one in the expected set
        InvalidMaturityStrategy,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::register_chain())]
        pub fn register_chain(
            origin: OriginFor<T>,
            chain_id: ChainId,
            chain_name: String,
            target_sample_size: Option<u32>,
            chain_attestation_interval: Option<u64>,
            attestation_checkpoint_interval: Option<u32>,
            max_attestors: Option<u32>,
            max_invulnerables: Option<u32>,
            attestation_chain_genesis_block_number: Option<u64>,
            vote_acceptance_window: Option<u64>,
            encoding: ChainEncodingVersion,
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
                    chain_encoding: encoding,
                    maturity_strategy: T::DefaultMaturityStrategy::get(),
                },
            );
            ChainIdAndNameToUniqKey::<T>::insert(
                chain_id,
                chain_name.as_bytes().to_vec(),
                chain_key,
            );

            ChainKeyValue::<T>::put(chain_key.checked_add(1).ok_or(Error::<T>::Arithmetic)?);

            T::ChainRegistrationHandler::on_register_chain(
                chain_key,
                chain_id,
                chain_name.as_bytes().to_vec(),
                target_sample_size,
                chain_attestation_interval,
                attestation_checkpoint_interval,
                max_attestors,
                max_invulnerables,
                attestation_chain_genesis_block_number,
                vote_acceptance_window,
                encoding,
            );

            Self::deposit_event(Event::ChainRegistered {
                chain_key,
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: encoding,
            });

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_chain())]
        pub fn remove_chain(
            origin: OriginFor<T>,
            chain_key: ChainId,
            remove_checkpoints: bool,
        ) -> DispatchResult {
            ensure_root(origin)?;

            let item = SupportedChains::<T>::get(chain_key).ok_or(Error::<T>::ChainNotSupported)?;

            ChainIdAndNameToUniqKey::<T>::remove(item.chain_id, item.chain_name.clone());

            SupportedChains::<T>::remove(chain_key);

            // Notify event listeners
            T::EventListeners::on_supported_chain_removed(chain_key, remove_checkpoints);

            Self::deposit_event(Event::ChainRemoved {
                chain_key,
                chain_id: item.chain_id,
                chain_name: item.chain_name.clone(),
                chain_encoding: item.chain_encoding,
            });

            Ok(())
        }

        /// Sets the maturity strategy for a supported chain
        /// Options
        /// - "EvmFinalized" Gets blocks once they are finalized
        /// - "EvmSafe" Gets blocks once they are confirmed
        /// - "EvmLatest" Gets blocks as soon as available
        /// - "FixedDelay: X" Gets blocks after they are X blocks old
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::set_maturity_strategy())]
        pub fn set_maturity_strategy(
            origin: OriginFor<T>,
            chain_key: ChainId,
            maturity_strategy: String,
        ) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(
                is_valid_maturity_strategy(&maturity_strategy),
                Error::<T>::InvalidMaturityStrategy
            );

            let mut item =
                SupportedChains::<T>::get(chain_key).ok_or(Error::<T>::ChainNotSupported)?;
            item.maturity_strategy = maturity_strategy;

            Self::deposit_event(Event::MaturityStrategySet {
                chain_key,
                chain_id: item.chain_id,
                chain_name: item.chain_name.clone(),
                maturity_strategy: item.maturity_strategy.clone(),
            });

            SupportedChains::<T>::insert(chain_key, item);

            Ok(())
        }
    }

    impl<T: Config> SupportedChainsProvider for Pallet<T> {
        fn is_chain_supported(chain_key: ChainKey) -> bool {
            SupportedChains::<T>::contains_key(chain_key)
        }

        fn supported_chains() -> Vec<ChainKey> {
            SupportedChains::<T>::iter()
                .map(|(chain_key, _)| chain_key)
                .collect::<Vec<ChainKey>>()
        }

        fn chain_key_by_chain_id_and_name(
            chain_id: ChainId,
            chain_name: Vec<u8>,
        ) -> Option<ChainId> {
            ChainIdAndNameToUniqKey::<T>::get(chain_id, chain_name)
        }

        fn get_supported_chain(chain_key: ChainKey) -> Option<SupportedChain> {
            SupportedChains::<T>::get(chain_key)
        }
    }
}

fn is_valid_maturity_strategy(strategy: &str) -> bool {
    match strategy {
        "EvmFinalized" | "EvmSafe" | "EvmLatest" => true,
        s if s.starts_with("FixedDelay:") => {
            // Split off the number part and trim whitespace
            let num_str = s.trim_start_matches("FixedDelay:").trim();
            // Try parsing as u64
            num_str.parse::<u64>().is_ok()
        }
        _ => false,
    }
}
