use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};
use sp_runtime::traits::Get;
use sp_std::marker::PhantomData;

/// Initializes `Operators` (pallet_membership Instance1) with an initial operator account.
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

                // Initial operator accounts
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
                // pallet_membership keeps `Members` sorted (its extrinsics binary-search).
                operators.sort();
                operators.dedup();

                let members: BoundedVec<T::AccountId, T::MaxMembers> = match BoundedVec::try_from(
                    operators,
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
        fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            frame_support::ensure!(
                Members::<T, OperatorsInstance>::get().len() == 4,
                "post_upgrade: expected exactly 4 operators after migration"
            );
            log::info!(
                target: "runtime::migrations",
                "v1_init_operators: post_upgrade checks passed"
            );
            Ok(())
        }
    }
}
