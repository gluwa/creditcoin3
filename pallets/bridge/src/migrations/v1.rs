use super::Migrate;
use super::Vec;
use crate::Config;
use crate::Pallet as PalletBridge;
use frame_support::pallet_prelude::*;
#[allow(unused_imports)]
use frame_support::storage::{migration::move_pallet, KeyPrefixIterator};
use frame_support::weights::Weight;
use sp_io::hashing::twox_128;

static OLD_PALLET_NAME: &str = "bridge";

pub(super) struct Migration<Runtime>(PhantomData<Runtime>);

impl<Runtime: Config> Migration<Runtime> {
    pub(super) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Config> Migrate for Migration<T> {
    fn pre_upgrade(&self) -> Vec<u8> {
        let new_pallet_name = PalletBridge::<T>::name();

        // make sure there are storage items in the old pallet
        let old_count = count_storage_items(OLD_PALLET_NAME);
        assert!(old_count != 0, "Storage items not found during migration");

        // make sure new pallet doesn't have any storage items
        let new_count = count_storage_items(new_pallet_name);
        assert!(new_count == 0, "New storage should be empty");

        // assert on storage version before migration starts
        assert!(<crate::Pallet<T> as GetStorageVersion>::on_chain_storage_version() < 1);

        old_count.to_le_bytes().to_vec()
    }

    fn migrate(&self) -> Weight {
        let new_pallet_name = PalletBridge::<T>::name();

        if OLD_PALLET_NAME.as_bytes() == new_pallet_name.as_bytes() {
            log::info!(
                    target: "runtime::PalletBridge",
                    "migrate V1, already migrated.",
            );
            return Weight::zero();
        }

        let old_count = count_storage_items(OLD_PALLET_NAME);
        assert_ne!(old_count, 0, "Old pallet storage must not be empty");

        move_pallet(OLD_PALLET_NAME.as_bytes(), new_pallet_name.as_bytes());

        // (1 read + 1 write) * <number of items>
        let mut weight: Weight = Weight::zero();
        let weight_each = T::DbWeight::get().reads_writes(1, 1);
        weight = weight.saturating_add(weight_each);
        weight.saturating_mul(old_count as u64)
    }

    fn post_upgrade(&self, ctx: Vec<u8>) {
        assert_eq!(
            StorageVersion::get::<crate::Pallet<T>>(),
            1,
            "expected storage version to be 1 after migrations complete"
        );

        let old_count = usize::from_le_bytes(ctx.try_into().unwrap());

        // make sure all storage items have been migrated
        let new_pallet_name = PalletBridge::<T>::name();
        let new_count = count_storage_items(new_pallet_name);
        assert_eq!(new_count, old_count, "Some storage items were not migrated");
    }
}

fn count_storage_items(pallet_name: &str) -> usize {
    let pallet_prefix = twox_128(pallet_name.as_bytes()).to_vec();
    let pallet_prefix_iter = frame_support::storage::KeyPrefixIterator::new(
        pallet_prefix.clone(),
        pallet_prefix,
        |key| Ok(key.to_vec()),
    );
    pallet_prefix_iter.count()
}

#[cfg(test)]
pub mod tests {
    use super::Migrate;
    use crate::mock::{ExtBuilder, Test};

    #[test]
    fn migrate_should_not_crash() {
        ExtBuilder.build_and_execute(|| {
            super::Migration::<Test>::new().migrate();
        });
    }
}
