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
    /// # Safety
    ///
    /// This function uses unsafe code to treat the entire structure as a byte slice.
    /// This is safe because of the `#[repr(C)]` attribute which ensures a predictable
    /// memory layout.
    #[inline]
    pub fn hash_all(&self) -> H::Output {
        unsafe {
            // Treat the prefix + hashes array as a contiguous byte slice and hash it directly.
            // Layout guaranteed by #[repr(C)] and homogeneous Output element type.
            let outputs_len = ARITY + 1;
            let byte_len = core::mem::size_of::<H::Output>() * outputs_len;
            let ptr = &self.prefix as *const H::Output as *const u8;
            let bytes = core::slice::from_raw_parts(ptr, byte_len);
            H::hash(bytes)
        }
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
