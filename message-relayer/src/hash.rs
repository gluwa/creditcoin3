//! `messageHash` builder.
//!
//! The implementation lives in the shared [`write_ability`] crate so the attestor (which produces
//! votes) and this relayer (which consumes them and builds the on-chain calldata) compute the hash
//! from a single source of truth. See [`write_ability::hash`] for the definition and unit vectors;
//! the integration oracle in `tests/golden_hash.rs` pins the byte layout against a hand-rolled
//! `keccak256(abi.encode(...))`.

pub use write_ability::hash::*;
