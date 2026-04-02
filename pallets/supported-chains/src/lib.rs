#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;
use supported_chains_primitives::{
    MATURITY_EVM_FINALIZED, MATURITY_EVM_LATEST, MATURITY_EVM_SAFE, MATURITY_FIXED_DELAY,
};

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

    /// The in-code storage version.
    const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        #[allow(deprecated)]
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
        /// Origin that can perform Operator-only calls
        type OperatorsOrigin: EnsureOrigin<Self::RuntimeOrigin>;
    }

    pub trait WeightInfo {
        fn register_chain() -> Weight;
        fn remove_chain() -> Weight;
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
            maturity_strategy: String,
        },

        /// A chain has been removed with a given ID
        ChainRemoved {
            chain_key: ChainKey,
            chain_id: ChainId,
            chain_name: Vec<u8>,
            chain_encoding: ChainEncodingVersion,
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
        /// Registers a supported chain with the given parameters. The chain key is automatically generated and returned in the ChainRegistered event.
        /// Only accounts in the Operators membership can call this extrinsic.
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
            encoding: ChainEncodingVersion,
            maturity_strategy: Option<String>,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                !ChainIdAndNameToUniqKey::<T>::contains_key(chain_id, chain_name.as_bytes()),
                Error::<T>::ChainAlreadyRegistered
            );

            let maturity_strategy = if let Some(strategy) = maturity_strategy {
                ensure!(
                    is_valid_maturity_strategy(&strategy),
                    Error::<T>::InvalidMaturityStrategy
                );
                strategy
            } else {
                T::DefaultMaturityStrategy::get()
            };

            let chain_key = ChainKeyValue::<T>::get();
            SupportedChains::<T>::insert(
                chain_key,
                SupportedChain {
                    chain_id,
                    chain_name: chain_name.as_bytes().to_vec(),
                    chain_encoding: encoding,
                    maturity_strategy: maturity_strategy.clone(),
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
                encoding,
            );

            Self::deposit_event(Event::ChainRegistered {
                chain_key,
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: encoding,
                maturity_strategy,
            });

            Ok(())
        }

        /// Removes a supported chain by its chain key. Only accounts in the Operators membership can call this extrinsic.
        #[pallet::call_index(1)]
        // We artificially add 200 writes to our assessed weight here since the `remove_chain` benchmark doesn't
        // account for modifications to storage made in pallet attestation poc. Actually capturing these writes
        // precisely requires dependence on about 10 additional pallets and 100's of added code lines. And we can afford
        // to charge more than necessary since we remove chains very rarely.
        #[pallet::weight(T::WeightInfo::remove_chain().saturating_add(T::DbWeight::get().writes(200)))]
        pub fn remove_chain(
            origin: OriginFor<T>,
            chain_key: ChainId,
            remove_checkpoints: bool,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

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
                maturity_strategy: item.maturity_strategy,
            });

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
        MATURITY_EVM_FINALIZED | MATURITY_EVM_SAFE | MATURITY_EVM_LATEST => true,
        s if s.starts_with(MATURITY_FIXED_DELAY) => {
            // Split off the number part and trim whitespace
            if let Some(num_str) = s.strip_prefix(MATURITY_FIXED_DELAY) {
                num_str.trim().parse::<u64>().is_ok()
            } else {
                false
            }
        }
        _ => false,
    }
}
