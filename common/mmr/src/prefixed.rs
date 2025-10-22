//! Prefixed hash storage for Merkle tree nodes.
//!
//! This module provides the `Prefixed` structure which stores a prefix along with
//! child node hashes in a contiguous memory block. The prefix is used to prevent
//! second-preimage attacks by domain separating leaf and inner node hashes.

use core::fmt::Debug;

use crate::traits::HashT;
use crate::ARITY;

/// Structure containing a prefix and hashes as a contiguous memory block.
///
/// The `#[repr(C)]` attribute ensures the layout is predictable, allowing safe
/// casting of the entire structure to a byte slice for hashing.
/// The prefix is used to prevent proof length extension attacks by domain-separating
/// leaf nodes from inner nodes.
#[repr(C)]
pub struct Prefixed<H: HashT> {
    prefix: H::Output,
    pub(crate) hashes: [H::Output; ARITY],
}

impl<H: HashT> Prefixed<H> {
    /// Returns the default hash value for empty leaves.
    ///
    /// This is the hash of an empty slice prefixed with `LEAF_HASH_PREPEND_VALUE`.
    #[inline]
    pub fn default_hash() -> H::Output {
        H::Output::default()
    }

    /// Hashes the prefix together with all child hashes.
    ///
    /// Only the conversion from a raw pointer to a byte slice is performed inside
    /// an `unsafe` block. The `#[repr(C)]` attribute and the homogeneous `H::Output`
    /// type are relied upon to make this access safe in practice. Keep the unsafe
    /// scope minimal and consider running miri when changing `H::Output` or memory layout.
    #[inline]
    pub fn hash_all(&self) -> H::Output {
        // Compute byte length for prefix + hashes
        let outputs_len = ARITY + 1;
        let byte_len = core::mem::size_of::<H::Output>() * outputs_len;

        // Only the slice construction from a raw pointer is unsafe; keep its scope minimal.
        let bytes: &[u8] = unsafe {
            let ptr = &self.prefix as *const H::Output as *const u8;
            core::slice::from_raw_parts(ptr, byte_len)
        };

        H::hash(bytes)
    }
}

impl<H: HashT> Clone for Prefixed<H> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<H: HashT> Copy for Prefixed<H> {}
impl<H: HashT> Default for Prefixed<H> {
    fn default() -> Self {
        Self {
            prefix: crate::INNER_HASH_PREPEND_VALUE.into(),
            hashes: [Self::default_hash(); ARITY],
        }
    }
}

impl<H: HashT> Debug for Prefixed<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        writeln!(f, "prefix: {:?}", self.prefix)?;
        for (i, h) in self.hashes.iter().enumerate() {
            writeln!(f, "h[{i}]: {h:?}")?;
        }
        Ok(())
    }
}
