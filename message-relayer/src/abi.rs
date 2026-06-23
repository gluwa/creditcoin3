//! Solidity ABI bindings used by the relayer.
//!
//! These live in the shared [`write_ability`] crate so the attestor (which decodes the same
//! `MessagePublished` event and recomputes the same `messageHash`) and this relayer cannot drift
//! apart. See [`write_ability::abi`] for the `sol!` declarations.

pub use write_ability::abi::*;
