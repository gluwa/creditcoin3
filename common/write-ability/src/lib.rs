//! Shared USC write-ability primitives.
//!
//! Both the attestor (vote producer) and the `message-relayer` (vote consumer / deliverer) must
//! agree, byte-for-byte, on how a cross-chain message is hashed, how a vote is framed on the wire,
//! which gossip topic carries it, and how the on-chain contracts are shaped. Keeping those
//! definitions in a single crate is the whole point of this module: if the attestor and relayer
//! ever diverged on any of them, signatures would verify as invalid on-chain (or peers would
//! never meet on the mesh) and delivery would silently fail.

pub mod abi;
pub mod envelope;
pub mod hash;
pub mod protocol;
