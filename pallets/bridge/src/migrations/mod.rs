use crate::{Config, Pallet};
use frame_support::{traits::StorageVersion, weights::Weight};
use sp_runtime::traits::UniqueSaturatedInto;
use sp_std::vec::Vec;

pub(crate) trait Migrate {
    fn pre_upgrade(&self) -> Vec<u8>;
    fn migrate(&self) -> Weight;
    fn post_upgrade(&self, blob: Vec<u8>);
}

mod v1;

pub(crate) fn migrate<T: Config>() -> Weight {
    let version = StorageVersion::get::<Pallet<T>>();
    let mut weight: Weight = Weight::zero();

    let callbacks: &[&dyn Migrate] = &[&v1::Migration::<T>::new()];

    for (idx, &calls) in callbacks.iter().enumerate() {
        let migration_idx = (idx + 1).unique_saturated_into();
        if version < migration_idx {
            #[cfg(feature = "try-runtime")]
            let blob = calls.pre_upgrade();
            weight.saturating_accrue(calls.migrate());
            StorageVersion::new(migration_idx).put::<Pallet<T>>();
            #[cfg(feature = "try-runtime")]
            calls.post_upgrade(blob);
        }
    }

    weight
}

#[cfg(test)]
pub mod tests {
    use super::{migrate, Weight};
    use crate::mock::{ExtBuilder, Test};

    #[test]
    fn migrate_should_not_crash() {
        ExtBuilder.build_and_execute(|| {
            let weight = migrate::<Test>();

            assert_ne!(weight, Weight::zero());
        });
    }
}
