use crate::traits::{HashT, ProofItemT, ProofValidator};
use crate::{Prefixed, ARITY};
use core::fmt::Debug;

extern crate alloc;
use alloc::{vec, vec::Vec};

/// Basic implementation of an item making up a proof.
/// Supports a power-of-2 number of siblings
pub struct ProofItem<H: HashT> {
    prefixed: Prefixed<H>,
    offset: usize,
}

impl<H: HashT> ProofItem<H> {
    /// returns item's hashes
    pub fn hashes(&self) -> &[H::Output; ARITY as usize] {
        &self.prefixed.hashes
    }

    /// returns item's offset
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<H: HashT> ProofItemT<H> for ProofItem<H> {
    /// constructor
    fn create(offset: usize, prefixed: Prefixed<H>) -> Self {
        Self { offset, prefixed }
    }
    /// hashes a provided hashed data at offset with its siblings
    fn hash_with_siblings(mut self, word_hash: H::Output) -> Option<H::Output> {
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
        writeln!(f, "{:?}", self.prefixed.hashes)
    }
}

/// Proof implementation the StaticTree generates
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

impl<H: HashT> ProofValidator for Proof<H> {
    /// verifies that the input was contained in the Merkle tree that generated this proof
    fn validate(&self, input: &[u8]) -> bool {
        let mut curr_hash = Some(Self::hash_as_leaf(input));

        // start from the base layer,
        // and for every item in the proof
        // put the hash derived from input into the proof item
        // at index stored in the proof item
        // and hash it with the siblings
        for item in self.items.iter() {
            curr_hash = curr_hash.and_then(|h| item.hash_with_siblings(h));
        }
        // validated iff the resulting hash is identical to the root
        curr_hash.as_ref() == Some(&self.root)
    }
}

impl<H: HashT> Proof<H> {
    /// returns the proof's length
    pub fn height(&self) -> usize {
        self.items.len()
    }

    /// returns the proof's root
    pub fn root(&self) -> H::Output {
        self.root
    }

    /// returns the proof's path
    pub fn path(&self) -> &[ProofItem<H>] {
        &self.items
    }

    /// returns the proof's arity
    pub fn arity() -> usize {
        ARITY as usize
    }

    /// prepends input with leaf prefix and hashes it
    pub fn hash_as_leaf(input: &[u8]) -> H::Output {
        let mut prefixed = vec![crate::LEAF_HASH_PREPEND_VALUE; input.len() + 1];
        prefixed[1..].copy_from_slice(input);

        H::hash(&prefixed[..])
    }
    /// returns the index of claim as tree's leaf
    pub fn claim_index(&self) -> usize {
        let mut a = 1usize;
        let mut index = 0;
        for item in self.items.iter() {
            index += a * item.offset();
            a *= ARITY as usize;
        }
        index
    }
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
