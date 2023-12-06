// First `LoanTerms` rework. `maturity` is replaced with `term_length`,
// and `InterestRate` changed from a type alias = u64 to a new struct `InterestRate`

use super::Migrate;
use super::{vec, Vec};
use crate::Config;
use frame_support::pallet_prelude::*;
use frame_support::weights::Weight;

pub(super) struct Migration<Runtime>(PhantomData<Runtime>);

impl<Runtime: Config> Migration<Runtime> {
    pub(super) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Config> Migrate for Migration<T> {
    fn pre_upgrade(&self) -> Vec<u8> {
        vec![]
    }

    fn migrate(&self) -> Weight {
        Weight::zero()
    }

    fn post_upgrade(&self, _ctx: Vec<u8>) {
        assert_eq!(
            StorageVersion::get::<crate::Pallet<T>>(),
            1,
            "expected storage version to be 1 after migrations complete"
        );
    }
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
