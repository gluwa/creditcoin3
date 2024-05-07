use core::fmt::Debug;
use core::slice::from_raw_parts;

use crate::traits::HashT;
use crate::ARITY;

/// structure containing a prefix (aligned to 4 bytes) and hashes as a contiguous memory block
/// prefix is used to prevent a proof length extension attack
#[repr(C)]
pub struct Prefixed<H: HashT> {
    prefix: H::Output,
    pub(crate) hashes: [H::Output; ARITY as usize],
}

impl<H: HashT> Prefixed<H> {
    /// hash of &[] prefixed with LEAF_HASH_PREPEND_VALUE
    #[inline]
    pub fn default_hash() -> H::Output {
        H::Output::default()
    }
    /// hash the prefix together with inner hashes
    #[inline]
    pub fn hash_all(&self) -> H::Output {
        unsafe {
            H::concat_then_hash(from_raw_parts(
                &self.prefix as *const <H as HashT>::Output,
                ARITY as usize + 1,
            ))
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
            hashes: [Self::default_hash(); ARITY as usize],
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
