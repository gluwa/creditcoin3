//! Runtime migrations.
//!
//! Live migrations (registered in the runtime's `SingleBlockMigrations` tuple in `lib.rs`):
//! the attest-coin migrations at the bottom of this file.
//!
//! The one-time genesis-init migration `v1_init_operators` is no longer registered: every
//! live network (devnet/testnet/mainnet) already has a non-empty operators membership, so
//! its data-absence guard can never fire again. It is kept for reference and historical
//! record only, hence its `dead_code` allow.

use frame_support::{
    dispatch::{DispatchResult, GetDispatchInfo},
    storage::migration,
    traits::OnRuntimeUpgrade,
    weights::Weight,
};
use sp_runtime::traits::{Dispatchable, Get, StaticLookup};
use sp_std::marker::PhantomData;

use crate::{
    attest_coin_precompile_account, AccountId, Balance, NativeOrEvmAddressLookup, Runtime,
    RuntimeCall, RuntimeOrigin, ATTEST_COIN_ASSET_ID,
};

/// Initializes `Operators` (pallet_membership Instance1) with the initial operator accounts.
///
/// Guards on data absence: runs if `Members` storage is empty, skips otherwise.
/// Data-absence (rather than version) guards are intentional — `BeforeAllRuntimeMigrations`
/// auto-syncs `StorageVersion` for new pallets before `OnRuntimeUpgrade` fires, so version-based
/// guards (`on_chain < in_code`) cannot be used for first-time pallet initialization.
#[allow(dead_code)]
pub mod v1_init_operators {
    use super::*;
    use frame_support::BoundedVec;
    use pallet_membership::Members;
    use sp_core::crypto::AccountId32;
    use sp_std::vec;

    type OperatorsInstance = pallet_membership::Instance1;

    pub struct Migration<T>(PhantomData<T>);

    /// The initial operator set, sorted and deduplicated.
    /// pallet_membership keeps `Members` sorted (its extrinsics binary-search).
    fn initial_operators<T>() -> sp_std::vec::Vec<T::AccountId>
    where
        T: pallet_membership::Config<OperatorsInstance>,
        T::AccountId: From<AccountId32>,
    {
        // 5ELVtGVj6BVa25EJWbUCvo44qWZ8389tPBB7d5dfGCfdbh9X
        // 5Eh2stFNQX4khuKoh2a1jQBVE91Lv3kyJiVP2Y5webontjRe
        // 5DzQB8D8cboKyvVqE1rUsGhwMUiFY71Qjc2sqWPV6Lr1V8nc
        // 5EiFZFResKra1gXUZ1KYXkj1aWdgr7Q78oZETCGrAjftnTTi
        let mut operators: sp_std::vec::Vec<T::AccountId> = vec![
            AccountId32::new(hex_literal::hex!(
                "648417311f63813098618f466b63227702ca140b26da0f96cc20367c169acd23"
            ))
            .into(),
            AccountId32::new(hex_literal::hex!(
                "742d54eb9c3cc4c3441a9bfaf9fc3869fd9e6e0cdf4222ece6bd4d8d1413d47b"
            ))
            .into(),
            AccountId32::new(hex_literal::hex!(
                "552ff68cef679a0543a0f20396bd09f808f2ca3ed304bb557dae5829da32eb5f"
            ))
            .into(),
            AccountId32::new(hex_literal::hex!(
                "751b41e92578e184661e790dee41ac2add7b3b7d9b019ccfc136926f5fabca56"
            ))
            .into(),
        ];
        operators.sort();
        operators.dedup();
        operators
    }

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

                let members: BoundedVec<T::AccountId, T::MaxMembers> = match BoundedVec::try_from(
                    initial_operators::<T>(),
                ) {
                    Ok(v) => v,
                    Err(_) => {
                        log::error!(
                            target: "runtime::migrations",
                            "v1_init_operators: MaxMembers too small for initial operators — skipping"
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
        fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
            use parity_scale_codec::Encode;

            Ok(Members::<T, OperatorsInstance>::get().encode())
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            use parity_scale_codec::Decode;

            let prior = sp_std::vec::Vec::<T::AccountId>::decode(&mut &state[..])
                .map_err(|_| "post_upgrade: failed to decode pre_upgrade members state")?;
            let current = Members::<T, OperatorsInstance>::get();

            if prior.is_empty() {
                frame_support::ensure!(
                    current.to_vec() == initial_operators::<T>(),
                    "post_upgrade: expected the initial operator set after migration"
                );
            } else {
                frame_support::ensure!(
                    current.to_vec() == prior,
                    "post_upgrade: pre-existing operator set was modified by the migration"
                );
            }
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

/// Reads `pallet_sudo::Key`, which is `StorageValue<_, T::AccountId, OptionQuery>`: the stored
/// value is a **bare** `AccountId`, with `None` represented by absence of the key rather than by
/// a SCALE `Option` prefix. Decoding as `Option<AccountId>` would read the account's first byte
/// as the enum discriminant and always yield `None`.
fn sudo_account() -> Option<AccountId> {
    migration::get_storage_value::<AccountId>(b"Sudo", b"Key", &[])
}

fn dispatch_root(call: RuntimeCall) -> DispatchResult {
    call.dispatch(RuntimeOrigin::root())
        .map(|_| ())
        .map_err(|e| e.error)
}

/// Sets `issuer` + `admin` to `precompile` (required for the attest-coin precompile's `mint` /
/// `burn`), leaving `owner` and `freezer` to the caller.
///
/// `owner` and `freezer` are separate parameters because `force_asset_status` takes them separately:
/// that lets the no-sudo path below preserve a split owner/freezer instead of collapsing both onto
/// one account.
fn apply_roles(
    precompile: &AccountId,
    owner: &AccountId,
    freezer: &AccountId,
    details: &pallet_assets::AssetDetails<Balance, AccountId, Balance>,
) -> Weight {
    let is_frozen = details.status == pallet_assets::AssetStatus::Frozen;
    let status = RuntimeCall::Assets(pallet_assets::Call::force_asset_status {
        id: ATTEST_COIN_ASSET_ID,
        owner: NativeOrEvmAddressLookup::unlookup(owner.clone()),
        issuer: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
        admin: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
        freezer: NativeOrEvmAddressLookup::unlookup(freezer.clone()),
        min_balance: details.min_balance,
        is_sufficient: details.is_sufficient,
        is_frozen,
    });

    let weight = status.get_dispatch_info().total_weight();
    if let Err(e) = dispatch_root(status) {
        log::error!(
            target: "runtime::migrations",
            "EnsureAttestCoinAssetRoles: force_asset_status failed: {e:?}"
        );
        return weight;
    }

    log::info!(
        target: "runtime::migrations",
        "EnsureAttestCoinAssetRoles: issuer+admin=precompile, owner={owner:?}, freezer={freezer:?}"
    );

    weight
}

/// Migrates pre-attest-coin attestor bonds (audit H-1 + H-2).
///
/// Before the attest-coin branch, attestor bonds locked native CTC on the stash via
/// `LockableCurrency::set_lock(BOND_LOCK_ID = b"b0ndl0ck")`. The branch deleted all lock
/// handling and re-denominated `Ledger.total_staked` as attest-coin (asset 1) held in the
/// shared bond pool. Without this migration, an upgraded chain is left with:
///
///   - stash CTC locked forever (no remaining code path removes `b0ndl0ck`), and
///   - legacy `Ledger` entries whose `total_staked` claims attest-coin that the pool does
///     not hold — so `withdraw_unbonded` / `kill` fail with `BondAssetTransferFailed`
///     indefinitely.
///
/// Strategy (per launch decision — only dev/test networks have live bonds, and mainnet's
/// bond requirement will be 0): clean break.
///
///   1. remove the legacy native-CTC lock from every `Ledger` stash (unlocks their CTC),
///   2. clear all legacy `Ledger` entries (attestors re-register and re-bond in attest-coin),
///   3. provision the bond pool account with a provider reference so the non-sufficient
///      asset can be transferred into it (first `register_attestor` would otherwise fail).
///
/// Idempotent: a cleared ledger map and an already-provisioned pool are both no-ops on
/// re-run. Guarded on ledger entries whose accounts still carry the legacy lock.
pub struct MigrateLegacyNativeBonds<T>(PhantomData<T>);

impl OnRuntimeUpgrade for MigrateLegacyNativeBonds<Runtime> {
    fn on_runtime_upgrade() -> Weight {
        use frame_support::traits::LockableCurrency;

        const LEGACY_BOND_LOCK_ID: [u8; 8] = *b"b0ndl0ck";

        let mut reads: u64 = 1;
        let mut writes: u64 = 0;

        // 3] Pool provisioning first — cheap, idempotent, and needed even on chains with no
        //    legacy ledgers at all (H-2: first bond on an upgraded chain fails otherwise).
        let pool: AccountId = crate::AttestationBondPoolAccount::get();
        reads = reads.saturating_add(1);
        if frame_system::Pallet::<Runtime>::providers(&pool) == 0 {
            frame_system::Pallet::<Runtime>::inc_providers(&pool);
            writes = writes.saturating_add(1);
            log::info!(
                target: "runtime::migrations",
                "MigrateLegacyNativeBonds: provisioned bond pool account with a provider ref"
            );
        }

        // 1] + 2] Legacy lock removal + ledger clearing. A ledger entry is "legacy" iff its
        //    stash still carries the old native-CTC bond lock — post-upgrade registrations
        //    never set that lock, so this guard makes the migration idempotent and safe to
        //    leave in the migration list across multiple upgrades.
        let stashes: sp_std::vec::Vec<AccountId> =
            pallet_attestation::Ledger::<Runtime>::iter_keys().collect();
        reads = reads.saturating_add(stashes.len() as u64);

        let mut migrated: u64 = 0;
        for stash in &stashes {
            let has_legacy_lock = pallet_balances::Locks::<Runtime>::get(stash)
                .iter()
                .any(|l| l.id == LEGACY_BOND_LOCK_ID);
            reads = reads.saturating_add(1);
            if !has_legacy_lock {
                continue;
            }

            <pallet_balances::Pallet<Runtime> as LockableCurrency<AccountId>>::remove_lock(
                LEGACY_BOND_LOCK_ID,
                stash,
            );
            pallet_attestation::Ledger::<Runtime>::remove(stash);
            // Registration placed a consumer reference on the stash (impls.rs `inc_consumers`);
            // mirror `kill_stash`'s `dec_consumers` so the account can be reaped once empty.
            frame_system::Pallet::<Runtime>::dec_consumers(stash);
            writes = writes.saturating_add(3);
            migrated = migrated.saturating_add(1);
        }

        if migrated > 0 {
            log::info!(
                target: "runtime::migrations",
                "MigrateLegacyNativeBonds: removed {migrated} legacy native bonds (locks + ledgers)"
            );
        }

        <Runtime as frame_system::Config>::DbWeight::get().reads_writes(reads, writes)
    }
}

/// Sets attest-coin asset roles: issuer + admin = precompile; owner + freezer = sudo.
///
/// If no sudo key is set (a chain that has run `Sudo::remove_key`), `owner` and `freezer` are each
/// left exactly as they are — a split between them included — rather than being pointed at the
/// precompile account. The precompile's account is not a keyed account, so assigning it `owner`
/// would leave the owner-only calls (`set_team`, `set_metadata`, `transfer_ownership`) with no holder
/// *and* make the idempotency check below pass forever, since the value compared against the stored
/// `owner` would be the precompile too. Root `force_asset_status` remains the repair path either
/// way; issuer/admin are still enforced because the precompile's `mint` / `burn` needs them.
pub struct EnsureAttestCoinAssetRoles<T>(PhantomData<T>);

impl OnRuntimeUpgrade for EnsureAttestCoinAssetRoles<Runtime> {
    fn on_runtime_upgrade() -> Weight {
        let precompile = attest_coin_precompile_account();
        let sudo = sudo_account();
        let db = <Runtime as frame_system::Config>::DbWeight::get();
        let mut weight = db.reads(2); // Sudo key and asset existence.

        if pallet_assets::Asset::<Runtime>::get(ATTEST_COIN_ASSET_ID).is_none() {
            let create = RuntimeCall::Assets(pallet_assets::Call::force_create {
                id: ATTEST_COIN_ASSET_ID,
                owner: NativeOrEvmAddressLookup::unlookup(precompile.clone()),
                is_sufficient: false,
                min_balance: 1u128,
            });
            weight = weight.saturating_add(create.get_dispatch_info().total_weight());
            if dispatch_root(create).is_err() {
                log::error!(
                    target: "runtime::migrations",
                    "EnsureAttestCoinAssetRoles: force_create failed for asset {ATTEST_COIN_ASSET_ID}"
                );
                return weight;
            }
        }

        weight = weight.saturating_add(db.reads(1));
        let Some(details) = pallet_assets::Asset::<Runtime>::get(ATTEST_COIN_ASSET_ID) else {
            log::error!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: asset {ATTEST_COIN_ASSET_ID} still missing after create"
            );
            return weight;
        };

        if details.status == pallet_assets::AssetStatus::Destroying {
            log::warn!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: asset {ATTEST_COIN_ASSET_ID} is Destroying; skipping"
            );
            return weight;
        }

        // Match fresh genesis on upgraded chains, including an asset created by an earlier
        // migration without metadata. Preserve any metadata already set by governance.
        weight = weight.saturating_add(db.reads(1));
        if !pallet_assets::Metadata::<Runtime>::contains_key(ATTEST_COIN_ASSET_ID) {
            let metadata = RuntimeCall::Assets(pallet_assets::Call::force_set_metadata {
                id: ATTEST_COIN_ASSET_ID,
                name: b"Attest Coin".to_vec(),
                symbol: b"AC".to_vec(),
                decimals: 18,
                is_frozen: false,
            });
            weight = weight.saturating_add(metadata.get_dispatch_info().total_weight());
            if let Err(e) = dispatch_root(metadata) {
                log::error!(
                    target: "runtime::migrations",
                    "EnsureAttestCoinAssetRoles: force_set_metadata failed: {e:?}"
                );
            }
        }

        // With no sudo key there is no governance account to install, so preserve whatever `owner`
        // and `freezer` are today — including a split between them — instead of inventing an unkeyed
        // one. `issuer`/`admin` are still enforced, since the precompile's `mint` / `burn` cannot
        // work without them.
        let (owner, freezer) = match &sudo {
            Some(sudo) => (sudo.clone(), sudo.clone()),
            None => {
                log::error!(
                    target: "runtime::migrations",
                    "EnsureAttestCoinAssetRoles: no sudo key set; leaving owner={:?} freezer={:?} \
                     unchanged and only enforcing issuer+admin=precompile. Governance must assign \
                     owner/freezer via root `force_asset_status`.",
                    details.owner,
                    details.freezer,
                );
                (details.owner.clone(), details.freezer.clone())
            }
        };

        if details.issuer == precompile
            && details.admin == precompile
            && details.owner == owner
            && details.freezer == freezer
        {
            log::info!(
                target: "runtime::migrations",
                "EnsureAttestCoinAssetRoles: roles already correct, skipping"
            );
            return weight;
        }

        weight.saturating_add(apply_roles(&precompile, &owner, &freezer, &details))
    }
}
