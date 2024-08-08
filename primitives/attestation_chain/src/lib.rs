use ethereum_types::U256;

mod dense_checkpoints;

pub mod attestation_checkpoints;
pub mod attestation_checkpoints_for_dev;
pub mod attestation_fragment;
pub mod block;
pub mod utils;

pub const CHECKPOINT_INTERVAL: usize = 10;
pub const FRAGMENT_SIZE: usize = CHECKPOINT_INTERVAL + 1;

//pub const ATTESTATION_GENESIS: u64 = 0;
//pub const ATTESTATION_GENESIS: u64 = 19605000;
pub const ATTESTATION_GENESIS: U256 = U256([0, 0, 0, 0]);

// #[cfg(not(test))]
// pub const ATTESTATION_GENESIS: u64 = 42;
