#![cfg_attr(not(feature = "std"), no_std)]

pub mod api;
pub mod provider;

pub const RANDOMNESS_LENGTH: usize = 32;

/// Randomness type required by BABE operations.
pub type Randomness = [u8; RANDOMNESS_LENGTH];

#[impl_trait_for_tuples::impl_for_tuples(10)]
pub trait OnRandomnessUpdate {
    fn on_new_epoch_randomness(_epoch: u64, randomness: Randomness);
}

/// Reports the worst-case weight a [`OnRandomnessUpdate`] listener consumes when
/// `pallet-randomness` notifies it of new epoch randomness.
///
/// Kept separate from [`OnRandomnessUpdate`] because that trait is derived for
/// tuples via full-automatic `impl_for_tuples`, which cannot generate a method
/// returning a non-unit value. `pallet-randomness` only invokes the listener on
/// an epoch boundary, so this lets the `on_initialize` hook charge for the
/// listener's epoch-change work (e.g. attestor election and interval updates)
/// from the listener's *own* benchmarked weights, rather than trying to measure
/// it through the randomness benchmark (which would run against an empty
/// attestor set and under-count).
pub trait OnRandomnessUpdateWeight {
    /// Worst-case weight consumed by `on_new_epoch_randomness`. Defaults to zero.
    fn on_new_epoch_randomness_weight() -> sp_weights::Weight {
        sp_weights::Weight::zero()
    }
}

/// Zero-weight listener: the unit type does no work, so it reports no weight.
impl OnRandomnessUpdateWeight for () {}
