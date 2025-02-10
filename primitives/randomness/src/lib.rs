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
