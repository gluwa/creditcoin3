use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};
use scale_info::prelude::string::String;
use sp_runtime::traits::Get;
use sp_std::marker::PhantomData;

/// Initializes `pallet_supported_chains` storage with the Ethereum chain.
///
/// Guards on data absence: runs if `SupportedChains` storage is empty, skips otherwise.
/// This is intentional — `BeforeAllRuntimeMigrations` auto-syncs `StorageVersion` for new
/// pallets before `OnRuntimeUpgrade` fires, so version-based guards (`on_chain < in_code`)
/// cannot be used for first-time pallet initialization.
pub mod v1_init_supported_chains {
    use super::*;
    use attestor_primitives::{ChainEncodingVersion, ChainKey};
    use pallet_supported_chains::{ChainIdAndNameToUniqKey, ChainKeyValue, SupportedChains};
    use supported_chains_primitives::{SupportedChain, MATURITY_EVM_SAFE};

    pub struct Migration<T>(PhantomData<T>);

    impl<T: pallet_supported_chains::Config> OnRuntimeUpgrade for Migration<T> {
        fn on_runtime_upgrade() -> Weight {
            if SupportedChains::<T>::iter().next().is_none() {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_supported_chains: running"
                );

                // Ethereum - chain_key 1
                let chain_key: ChainKey = 1;
                let chain_id: u64 = 1;
                let chain_name = "Ethereum".as_bytes().to_vec();

                SupportedChains::<T>::insert(
                    chain_key,
                    SupportedChain {
                        chain_id,
                        chain_name: chain_name.clone(),
                        chain_encoding: ChainEncodingVersion::V1,
                        maturity_strategy: String::from(MATURITY_EVM_SAFE),
                    },
                );
                ChainIdAndNameToUniqKey::<T>::insert(chain_id, chain_name, chain_key);
                // Next available chain_key = 2 (one chain inserted, counter starts at 1)
                ChainKeyValue::<T>::put(2u64);

                log::info!(
                    target: "runtime::migrations",
                    "v1_init_supported_chains: complete"
                );

                T::DbWeight::get().reads_writes(1, 3)
            } else {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_supported_chains: skipping (already initialized)"
                );
                T::DbWeight::get().reads(1)
            }
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            let chain_key: attestor_primitives::ChainKey = 1;
            frame_support::ensure!(
                SupportedChains::<T>::contains_key(chain_key),
                "post_upgrade: chain_key=1 (Ethereum) not found in SupportedChains"
            );
            frame_support::ensure!(
                ChainKeyValue::<T>::get() == 2u64,
                "post_upgrade: ChainKeyValue is not 2"
            );
            log::info!(
                target: "runtime::migrations",
                "v1_init_supported_chains: post_upgrade checks passed"
            );
            Ok(())
        }
    }
}

/// Initializes `pallet_attestation_poc` storage for chain_key=1 (Ethereum).
///
/// Guards on data absence: runs if `TargetSampleSize` has no entry for chain_key=1.
/// See `v1_init_supported_chains` for explanation of why version-based guards cannot be used.
pub mod v1_init_attestation {
    use super::*;
    use attestor_primitives::ChainKey;
    use pallet_attestation::{
        AttestationCheckpointInterval, AttestorElectionPolicy, ChainAttestationInterval,
        ChainElectionPolicy, MaxAttestors, MaxInvulnerables, TargetSampleSize,
    };

    pub struct Migration<T>(PhantomData<T>);

    impl<T: pallet_attestation::Config> OnRuntimeUpgrade for Migration<T> {
        fn on_runtime_upgrade() -> Weight {
            let chain_key: ChainKey = 1;

            if !TargetSampleSize::<T>::contains_key(chain_key) {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_attestation: running"
                );

                TargetSampleSize::<T>::insert(chain_key, 9u32);
                ChainAttestationInterval::<T>::insert(chain_key, 10u64);
                AttestationCheckpointInterval::<T>::insert(chain_key, 10u32);
                MaxAttestors::<T>::insert(chain_key, T::MaxAttestationNodes::get());
                MaxInvulnerables::<T>::insert(chain_key, T::MaxAttestationNodes::get());
                ChainElectionPolicy::<T>::insert(chain_key, AttestorElectionPolicy::AuthorizedOnly);

                log::info!(
                    target: "runtime::migrations",
                    "v1_init_attestation: complete"
                );

                T::DbWeight::get().reads_writes(1, 5)
            } else {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_attestation: skipping (already initialized)"
                );
                T::DbWeight::get().reads(1)
            }
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            let chain_key: ChainKey = 1;
            frame_support::ensure!(
                TargetSampleSize::<T>::contains_key(chain_key),
                "post_upgrade: TargetSampleSize not set for chain_key=1"
            );
            frame_support::ensure!(
                ChainAttestationInterval::<T>::contains_key(chain_key),
                "post_upgrade: ChainAttestationInterval not set for chain_key=1"
            );
            frame_support::ensure!(
                AttestationCheckpointInterval::<T>::contains_key(chain_key),
                "post_upgrade: AttestationCheckpointInterval not set for chain_key=1"
            );
            frame_support::ensure!(
                MaxAttestors::<T>::contains_key(chain_key),
                "post_upgrade: MaxAttestors not set for chain_key=1"
            );
            frame_support::ensure!(
                MaxInvulnerables::<T>::contains_key(chain_key),
                "post_upgrade: MaxInvulnerables not set for chain_key=1"
            );
            log::info!(
                target: "runtime::migrations",
                "v1_init_attestation: post_upgrade checks passed"
            );
            Ok(())
        }
    }
}

/// Initializes `Operators` (pallet_membership Instance1) with an initial operator account.
///
/// Guards on data absence: runs if `Members` storage is empty, skips otherwise.
/// See `v1_init_supported_chains` for explanation of why version-based guards cannot be used.
pub mod v1_init_operators {
    use super::*;
    use frame_support::BoundedVec;
    use pallet_membership::Members;
    use sp_core::crypto::AccountId32;
    use sp_std::vec;

    type OperatorsInstance = pallet_membership::Instance1;

    pub struct Migration<T>(PhantomData<T>);

    impl<T: pallet_membership::Config<OperatorsInstance>> OnRuntimeUpgrade for Migration<T>
    where
        T::AccountId: From<AccountId32>,
    {
        fn on_runtime_upgrade() -> Weight {
            if Members::<T, OperatorsInstance>::get().is_empty() {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_operators: running"
                );

                // Initial operator account — can be removed or supplemented via governance.
                // 5Co5nmjuasULHzwBouuNZ1wYNKjHiBXubDY6WQz5ep2zHTDc
                let operator = AccountId32::new(hex_literal::hex!(
                    "205223b1acdf381019ceedd2a65197b95769b965f67a7693c924536e3b394047"
                ));

                let members: BoundedVec<T::AccountId, T::MaxMembers> = match BoundedVec::try_from(
                    vec![operator.into()],
                ) {
                    Ok(v) => v,
                    Err(_) => {
                        log::error!(
                            target: "runtime::migrations",
                            "v1_init_operators: MaxMembers is 0 — skipping operator initialization"
                        );
                        return T::DbWeight::get().reads(1);
                    }
                };
                Members::<T, OperatorsInstance>::put(members);

                log::info!(
                    target: "runtime::migrations",
                    "v1_init_operators: complete"
                );

                T::DbWeight::get().reads_writes(1, 1)
            } else {
                log::info!(
                    target: "runtime::migrations",
                    "v1_init_operators: skipping (already initialized)"
                );
                T::DbWeight::get().reads(1)
            }
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            frame_support::ensure!(
                !Members::<T, OperatorsInstance>::get().is_empty(),
                "post_upgrade: Operators Members storage is empty after migration"
            );
            log::info!(
                target: "runtime::migrations",
                "v1_init_operators: post_upgrade checks passed"
            );
            Ok(())
        }
    }
}
