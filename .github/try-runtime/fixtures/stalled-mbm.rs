//! try-runtime self-test fixture: a multi-block migration that never finishes.
//!
//! Never part of the real runtime. `.github/try-runtime/fixtures/apply.sh` injects it
//! into `pallet_migrations::Config::Migrations` for
//! `.github/workflows/try-runtime-selftest.yml`, which expects the multi-block pass of
//! the try-runtime check to fail on its `--mbm-max-blocks` bound.
//!
//! `pallet_migrations` takes at most one step of a migration per block, and this one
//! always asks for another, with no `max_steps` to fail it first, so the migration stays
//! ongoing for as many blocks as the CLI simulates.

use frame_support::{
    migrations::{SteppedMigration, SteppedMigrationError},
    weights::WeightMeter,
};

pub type Fixture = StalledMigration;

pub struct StalledMigration;

impl SteppedMigration for StalledMigration {
    type Cursor = u32;
    type Identifier = [u8; 24];

    fn id() -> Self::Identifier {
        *b"try-runtime-selftest-mbm"
    }

    fn step(
        cursor: Option<Self::Cursor>,
        _meter: &mut WeightMeter,
    ) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
        let step = cursor.unwrap_or_default().saturating_add(1);
        log::debug!("try-runtime selftest: stalled-mbm fixture at step {step}");
        Ok(Some(step))
    }
}
