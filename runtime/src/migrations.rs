// Runtime migrations

pub mod staking_v13_to_v15 {
    use frame_support::{
        pallet_prelude::*,
        traits::{Get, UncheckedOnRuntimeUpgrade},
        weights::Weight,
    };
    use pallet_staking::SessionInterface;
    use sp_std::vec::Vec;

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
    }

    pub type MigrateV13ToV15<T> = frame_support::migrations::VersionedMigration<
        13,
        15,
        VersionUncheckedMigrateV13ToV15<T>,
        pallet_staking::Pallet<T>,
        <T as frame_system::Config>::DbWeight,
    >;
}
