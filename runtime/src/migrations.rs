// Runtime migrations

pub mod staking_v13_to_v15 {
    use frame_support::{
        pallet_prelude::*,
        traits::{Get, UncheckedOnRuntimeUpgrade},
        weights::Weight,
    };
    use pallet_staking::SessionInterface;
    use sp_std::vec::Vec;

    #[cfg(feature = "try-runtime")]
    use sp_runtime::TryRuntimeError;

    type DefaultDisablingStrategy = pallet_staking::UpToLimitDisablingStrategy;

    #[frame_support::storage_alias]
    pub(crate) type OffendingValidatorsV13<T: pallet_staking::Config> =
        StorageValue<pallet_staking::Pallet<T>, Vec<(u32, bool)>, ValueQuery>;

    #[frame_support::storage_alias]
    pub(crate) type DisabledValidators<T: pallet_staking::Config> =
        StorageValue<pallet_staking::Pallet<T>, Vec<u32>, ValueQuery>;

    pub struct VersionUncheckedMigrateV13ToV15<T>(core::marker::PhantomData<T>);
    impl<T: pallet_staking::Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV13ToV15<T> {
        fn on_runtime_upgrade() -> Weight {
            // Read the old storage format
            let mut migrated = OffendingValidatorsV13::<T>::take()
                .into_iter()
                .filter(|p| p.1) // take only disabled validators
                .map(|p| p.0)
                .collect::<Vec<_>>();

            // Respect disabling limit
            migrated.truncate(DefaultDisablingStrategy::disable_limit(
                T::SessionInterface::validators().len(),
            ));

            // Write to new storage
            DisabledValidators::<T>::set(migrated);

            log::info!("Staking v13->v15 migration applied successfully.");
            T::DbWeight::get().reads_writes(2, 2)
        }

        #[cfg(feature = "try-runtime")]
        fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
            use codec::Encode;

            let offending_count = OffendingValidatorsV13::<T>::get().len();
            log::info!(
                "Pre-upgrade: Found {} offending validators",
                offending_count
            );

            Ok(offending_count.encode())
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
            use codec::Decode;

            frame_support::ensure!(
                OffendingValidatorsV13::<T>::decode_len().is_none(),
                "OffendingValidators (old format) is not empty after migration"
            );

            let pre_count =
                usize::decode(&mut &state[..]).map_err(|_| "Failed to decode pre-upgrade state")?;
            let post_count = DisabledValidators::<T>::get().len();

            log::info!(
                "Post-upgrade: Migrated {} validators, now {} disabled validators",
                pre_count,
                post_count
            );

            Ok(())
        }
    }

    pub type MigrateV13ToV15<T> = frame_support::migrations::VersionedMigration<
        13,
        15,
        VersionUncheckedMigrateV13ToV15<T>,
        pallet_staking::Pallet<T>,
        <T as frame_system::Config>::DbWeight,
    >;
}
