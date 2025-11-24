#![cfg_attr(not(feature = "std"), no_std)]

/// Cost of each storage read (matches cold SLOAD) in gas.
pub const GAS_STORAGE_LOOKUP: u64 = 2_600;

/// Per item processed in iteration
pub const GAS_PER_ITERATION_ITEM: u64 = 26;
