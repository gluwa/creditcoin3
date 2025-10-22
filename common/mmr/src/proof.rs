//! Proof generation and verification for Merkle trees.
//!
//! This module provides structures for creating and validating Merkle proofs,
//! which allow efficient verification that a specific leaf exists in a tree
//! without needing to reconstruct the entire tree.

use crate::traits::HashT;
use crate::{Prefixed, ARITY};
use core::fmt::Debug;

extern crate alloc;
use alloc::{vec, vec::Vec};

/// A single item in a Merkle proof path.
///
/// Contains the sibling hashes at a specific level of the tree and the offset
/// indicating which position the proven leaf occupies among its siblings.
/// Supports a power-of-2 number of siblings.
pub struct ProofItem<H: HashT> {
    prefixed: Prefixed<H>,
    offset: usize,
}

impl<H: HashT> ProofItem<H> {
    /// Returns the sibling hashes at this level
    pub fn hashes(&self) -> &[H::Output; ARITY] {
        &self.prefixed.hashes
    }

    /// Returns the offset indicating the position among siblings
    pub fn offset(&self) -> usize {
        self.offset
    }
}

// Removed ProofItemT trait implementation (trait no longer exists)
impl<H: HashT> ProofItem<H> {
    pub(crate) fn create(offset: usize, prefixed: Prefixed<H>) -> Self {
        Self { offset, prefixed }
    }
    pub(crate) fn hash_with_siblings(mut self, word_hash: H::Output) -> Option<H::Output> {
        self.prefixed.hashes[self.offset] = word_hash;
        Some(self.prefixed.hash_all())
    }
}

impl<H: HashT> Copy for ProofItem<H> {}

impl<H: HashT> Clone for ProofItem<H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H: HashT> Default for ProofItem<H> {
    fn default() -> Self {
        Self {
            prefixed: Default::default(),
            offset: Default::default(),
        }
    }
}

impl<H: HashT> Debug for ProofItem<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        writeln!(f, "{:?}@{}", self.prefixed.hashes, self.offset)
    }
}

/// A Merkle proof that can verify a leaf's inclusion in a tree.
///
/// Contains the root hash and a path of proof items from the leaf to the root.
/// The proof can be validated against input data to confirm it was part of the
/// original tree that produced the given root hash.
pub struct Proof<H: HashT> {
    root: H::Output,
    items: Vec<ProofItem<H>>,
}

impl<H: HashT> Proof<H> {
    pub(crate) fn from_root(root: H::Output) -> Self {
        Self {
            root,
            items: vec![],
        }
    }

    pub(crate) fn push(&mut self, offset: usize, prefixed: Prefixed<H>) {
        self.items.push(ProofItem::<H>::create(offset, prefixed));
    }
}

impl<H: HashT> Proof<H> {
    /// Verifies that the input was contained in the Merkle tree that generated this proof
    pub fn validate(&self, input: &[u8]) -> bool {
        let mut curr_hash = Some(Self::hash_as_leaf(input));
        for item in self.items.iter() {
            curr_hash = curr_hash.and_then(|h| item.hash_with_siblings(h));
        }
        curr_hash.as_ref() == Some(&self.root)
    }

    /// Lightweight serialization helper returning a plain, allocation-friendly representation.
    pub fn serialize(&self) -> SerializedProof<H> {
        SerializedProof {
            root: self.root,
            height: self.height(),
            items: self
                .items
                .iter()
                .map(|pi| SerializedProofItem {
                    offset: pi.offset(),
                    hashes: pi.hashes().to_vec(),
                })
                .collect(),
        }
    }
}

/// A serializable proof item (hashes + offset)
pub struct SerializedProofItem<H: HashT> {
    pub offset: usize,
    pub hashes: Vec<H::Output>,
}

/// A serializable view of a Merkle proof
pub struct SerializedProof<H: HashT> {
    pub root: H::Output,
    pub height: usize,
    pub items: Vec<SerializedProofItem<H>>,
}

impl<H: HashT> Proof<H> {
    /// Returns the proof's height (number of levels)
    pub fn height(&self) -> usize {
        self.items.len()
    }

    /// Returns the proof's root hash
    pub fn root(&self) -> H::Output {
        self.root
    }

    /// Returns the proof's path (sequence of proof items)
    pub fn path(&self) -> &[ProofItem<H>] {
        &self.items
    }

    /// Prepends input with leaf prefix and hashes it
    pub fn hash_as_leaf(input: &[u8]) -> H::Output {
        let mut prefixed = vec![crate::LEAF_HASH_PREPEND_VALUE; input.len() + 1];
        prefixed[1..].copy_from_slice(input);

        H::hash(&prefixed[..])
    }

    #[cfg(test)]
    /// Chains this proof with another proof, extending the path to the root
    pub fn chain(mut self, other: Self) -> Self {
        if other.height() > 0 {
            self.root = other.root();
            self.items.extend(other.items);
        }
        self
    }
}

impl<H: HashT> Default for Proof<H> {
    fn default() -> Self {
        Self {
            root: Default::default(),
            items: vec![],
        }
    }
}

impl<H: HashT> Debug for Proof<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        writeln!(f, "[proof height]:   {:?}", self.height())?;
        writeln!(f, "[proof root]:   {:?}", self.root)?;
        write!(f, "{:?}", self.items)
    }
}
