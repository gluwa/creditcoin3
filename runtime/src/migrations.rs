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
