//! One-time genesis-init migrations.
//!
//! These have already run on every network (devnet/testnet/mainnet), so they are no longer
//! registered in the runtime's `SingleBlockMigrations` tuple (see `lib.rs`). They are kept
//! here for reference and historical record only. The `Migration` structs are therefore
//! intentionally never constructed, hence the module-wide `dead_code` allow.
#![allow(dead_code)]

use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};
use sp_runtime::traits::Get;
use sp_std::marker::PhantomData;

/// Initializes `Operators` (pallet_membership Instance1) with the initial operator accounts.
///
/// Guards on data absence: runs if `Members` storage is empty, skips otherwise.
/// Data-absence (rather than version) guards are intentional — `BeforeAllRuntimeMigrations`
/// auto-syncs `StorageVersion` for new pallets before `OnRuntimeUpgrade` fires, so version-based
/// guards (`on_chain < in_code`) cannot be used for first-time pallet initialization.
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
