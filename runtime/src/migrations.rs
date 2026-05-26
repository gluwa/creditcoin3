use frame_support::{
    dispatch::DispatchResult, pallet_prelude::Blake2_128Concat, storage::migration, storage_alias,
    traits::OnRuntimeUpgrade, weights::Weight,
};
use parity_scale_codec::{Decode, Encode};
use scale_info::prelude::string::String;
use sp_runtime::traits::{Dispatchable, Get, StaticLookup};
use sp_std::marker::PhantomData;

use crate::{
    attest_coin_precompile_account, AccountId, Balance, NativeOrEvmAddressLookup, Runtime,
    RuntimeCall, RuntimeOrigin, ATTEST_COIN_ASSET_ID,
};

// Used only by `#[storage_alias]` expansion (rustc does not see it as a normal use).
#[allow(unused_imports)]
use crate::Assets as AssetsPallet;

/// Initializes `pallet_supported_chains` storage with the Sepolia Ethereum chain.
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

                // Sepolia Ethereum - chain_key 1
                let chain_key: ChainKey = 1;
                let chain_id: u64 = 11155111;
                let chain_name = "Sepolia ethereum".as_bytes().to_vec();

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
                "post_upgrade: chain_key=1 (Sepolia) not found in SupportedChains"
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

/// Initializes `pallet_attestation_poc` storage for chain_key=1 (Sepolia).
///
/// Guards on data absence: runs if `TargetSampleSize` has no entry for chain_key=1.
/// See `v1_init_supported_chains` for explanation of why version-based guards cannot be used.
pub mod v1_init_attestation {
    use super::*;
    use attestor_primitives::ChainKey;
    use pallet_attestation::{
        AttestationCheckpointInterval, ChainAttestationInterval, MaxAttestors, MaxInvulnerables,
        TargetSampleSize,
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
                // No invulnerables or checkpoints for initial config.

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
                // 5HbPgFzxtmmMvonZHL7ykepUqN8cnMFgWci2SRJ6LHMt8dcb
                let operator = AccountId32::new(hex_literal::hex!(
                    "f49493c655bf40a6af007f4f6285f5bf71a8925893b93b4c6526c6c7e874cd47"
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

// --- Attest coin (`pallet-assets` id 1): issuer + admin = precompile (mint/deposit + burn/withdraw);
//     owner + freezer = sudo or precompile (governance ops; admin must be precompile for precompile `burn`).

/// Mirrors [`pallet_assets::types::AssetDetails`] / asset status SCALE layout so we can decode
/// storage without relying on `pub(super)` field access in the pallet.
#[derive(Decode, Encode, Eq, PartialEq)]
enum MirrorAssetStatus {
    Live,
    Frozen,
    Destroying,
}

#[derive(Decode, Encode)]
struct MirrorAssetDetails {
    owner: AccountId,
    issuer: AccountId,
    admin: AccountId,
    freezer: AccountId,
    supply: Balance,
    deposit: Balance,
    min_balance: Balance,
    is_sufficient: bool,
    accounts: u32,
    sufficients: u32,
    approvals: u32,
    status: MirrorAssetStatus,
}

#[storage_alias]
type AssetMap = StorageMap<AssetsPallet, Blake2_128Concat, u32, MirrorAssetDetails>;

fn sudo_account() -> Option<AccountId> {
    migration::get_storage_value::<Option<AccountId>>(b"Sudo", b"Key", &[]).flatten()
}

fn dispatch_root(call: RuntimeCall) -> DispatchResult {
    call.dispatch(RuntimeOrigin::root())
        .map(|_| ())
        .map_err(|e| e.error)
}

fn apply_roles(
    precompile: &AccountId,
    governance: &AccountId,
    details: &MirrorAssetDetails,
) -> Weight {
    let is_frozen = details.status == MirrorAssetStatus::Frozen;
    let status = RuntimeCall::Assets(pallet_assets::Call::force_asset_status {
        id: ATTEST_COIN_ASSET_ID,
        owner: NativeOrEvmAddressLookup::unlookup(governance.clone()),
        issuer: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
        admin: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
        freezer: NativeOrEvmAddressLookup::unlookup(governance.clone()),
        min_balance: details.min_balance,
        is_sufficient: details.is_sufficient,
        is_frozen,
    });

    if let Err(e) = dispatch_root(status) {
        log::error!(
            target: "runtime::migrations",
            "EnsureAttestCoinAssetRoles: force_asset_status failed: {e:?}"
        );
        return <Runtime as frame_system::Config>::DbWeight::get().reads_writes(3, 0);
    }

    log::info!(
        target: "runtime::migrations",
        "EnsureAttestCoinAssetRoles: issuer+admin=precompile, owner+freezer=governance"
    );

    <Runtime as frame_system::Config>::DbWeight::get().reads_writes(4, 4)
}

/// Sets attest-coin asset roles: issuer + admin = precompile; owner + freezer = sudo or precompile.
pub struct EnsureAttestCoinAssetRoles<T>(PhantomData<T>);

impl OnRuntimeUpgrade for EnsureAttestCoinAssetRoles<Runtime> {
    fn on_runtime_upgrade() -> Weight {
        let precompile = attest_coin_precompile_account();
        let governance = sudo_account().unwrap_or_else(|| precompile.clone());

        if AssetMap::get(ATTEST_COIN_ASSET_ID).is_none() {
            let create = RuntimeCall::Assets(pallet_assets::Call::force_create {
                id: ATTEST_COIN_ASSET_ID,
                owner: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
                is_sufficient: false,
                min_balance: 1u128,
            });
            if dispatch_root(create).is_err() {
                log::error!(
                    target: "runtime::migrations",
                    "EnsureAttestCoinAssetRoles: force_create failed for asset {ATTEST_COIN_ASSET_ID}"
                );
                return <Runtime as frame_system::Config>::DbWeight::get().reads_writes(2, 0);
            }
        }

        let Some(details) = AssetMap::get(ATTEST_COIN_ASSET_ID) else {
            log::error!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: asset {ATTEST_COIN_ASSET_ID} still missing after create"
            );
            return <Runtime as frame_system::Config>::DbWeight::get().reads_writes(4, 2);
        };

        if details.status == MirrorAssetStatus::Destroying {
            log::warn!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: asset {ATTEST_COIN_ASSET_ID} is Destroying; skipping"
            );
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }

        if details.issuer == precompile
            && details.admin == precompile
            && details.owner == governance
            && details.freezer == governance
        {
            log::info!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: roles already correct, skipping"
            );
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }

        apply_roles(&precompile, &governance, &details)
    }
}
