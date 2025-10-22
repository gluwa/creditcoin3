//! A Merkle tree implementation with binary arity for Starknet Pedersen hashing.
//!
//! This crate provides an efficient implementation of binary Merkle trees with support for:
//! - Proof generation and verification
//! - Fixed binary arity (2 children per node)
//! - Configurable hash function via the `HashT` trait
//!
//! # Primary Use Case
//!
//! This crate is used to create Merkle trees of Ethereum block data using Starknet Pedersen hash.
//! The trees are used for attestation and proof generation in the Creditcoin attestor system.
//!
//! # Examples
//!
//! ```ignore
//! use mmr::{BaseTree, traits::MerkleTreeTrait};
//! use utils::StarknetPedersenMerkleTree;
//!
//! // Create a tree from block data
//! let data = vec![b"leaf1", b"leaf2", b"leaf3"];
//! let tree = StarknetPedersenMerkleTree::from(&data[..]);
//!
//! // Generate a proof for the first leaf
//! let proof = tree.generate_proof(0);
//!
//! // Get the root hash
//! let root = tree.root();
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

mod prefixed;
pub mod proof;
#[cfg(test)]
mod tests;
pub mod traits;
mod utils;

use core::fmt::Debug;
use core::mem::size_of;
use core::ops::Deref;

extern crate alloc;
use alloc::{vec, vec::Vec};

use crate::prefixed::Prefixed;
use crate::proof::Proof;
use crate::traits::HashT;
use crate::utils::{height, layer_size, location_in_prefixed, num_of_prefixed_for_input};

pub const ARITY: usize = 2;

/// Leaves will be prepended with this value prior to hashing
pub const LEAF_HASH_PREPEND_VALUE: u8 = 0;
/// Inner nodes will be prepended with this value prior to hashing
pub const INNER_HASH_PREPEND_VALUE: u8 = 1;

/// The arity (branching factor) of the Merkle tree.
///
/// This determines how many children each node has. Only power-of-2 arities are supported
/// for efficient bit manipulation during tree traversal.
/// NOTE: Arity enum removed; binary arity fixed at 2 via `pub const ARITY: usize = 2;`
/// A complete Merkle tree with a fixed arity.
///
/// This structure stores all tree nodes in a compact format using prefixed hash blocks.
/// The tree is constructed from leaf data and supports proof generation and leaf updates.
pub struct BaseTree<H: HashT> {
    root: H::Output,
    prefixed: Vec<Prefixed<H>>,
    height: usize,
    num_of_leaves: usize,
}

impl<H, T> From<&[T]> for BaseTree<H>
where
    H: HashT,
    T: AsRef<[u8]> + Deref<Target = [u8]> + Debug,
{
    fn from(input: &[T]) -> Self {
        let len = input.len();
        let max_len = input.iter().map(|d| d.len()).max().unwrap_or(0);
        let mut prefixed_input = vec![LEAF_HASH_PREPEND_VALUE; max_len + 1];

        let mut this = Self::default_alloc(len);

        // fill the base layer
        for (i, d) in input.iter().enumerate() {
            prefixed_input[1..d.len() + 1].copy_from_slice(d.as_ref());

            let (index, offset) = location_in_prefixed(i, ARITY);
            this.prefixed[index].hashes[offset] = H::hash(&prefixed_input[0..d.len() + 1]);
        }

        this.pad_leaves(len);
        // fill the rest of layers
        this.fill_layers(len);
        this
    }
}

impl<H: HashT> BaseTree<H> {
    const ARITY: usize = ARITY;

    #[cfg(test)]
    fn from_leaves(leaves: &[H::Output]) -> Self {
        let len = leaves.len();

        let mut this = Self::default_alloc(len);

        for (i, leaf) in leaves.iter().enumerate() {
            let (index, offset) = location_in_prefixed(i, ARITY);
            this.prefixed[index].hashes[offset] = *leaf;
        }
        this.pad_leaves(len);
        // fill the rest of layers
        this.fill_layers(len);
        this
    }

    fn default_alloc(len: usize) -> Self {
        Self {
            root: Default::default(),
            prefixed: vec![Prefixed::default(); num_of_prefixed_for_input(len, ARITY)],
            height: height(len, ARITY),
            num_of_leaves: len,
        }
    }

    fn base_layer_size(&self) -> usize {
        layer_size(ARITY, self.height, 0)
    }

    pub fn height(&self) -> usize {
        self.height
    }

    #[inline]
    fn align_len(len: usize, arity: usize) -> usize {
        // len / arity + (len mod arity)
        (len >> arity.trailing_zeros() as usize) + usize::from(len & (arity - 1) != 0)
    }

    fn fill_layers(&mut self, data_len: usize) {
        debug_assert!(
            !self.prefixed.is_empty(),
            "prefixed buffer must be non-empty"
        );
        if self.height == 0 {
            self.root = self.prefixed[0].hashes[0];
            return;
        }

        let mut start_ind = 0;
        let mut next_layer_ind = self.base_layer_size();
        let mut data_len_aligned = Self::align_len(data_len, Self::ARITY);

        for h in 0..self.height - 1 {
            // hash packed siblings of the current layer and fill the upper layer
            for i in start_ind..data_len_aligned {
                debug_assert!(
                    i < self.prefixed.len(),
                    "index out of bounds in fill_layers (base hash loop)"
                );
                let offset = i & (Self::ARITY - 1); // index modulo ARITY
                let (j, _) = self.parent_index_and_base(i, h, start_ind);

                // hash concatenated siblings from the contiguous memory
                // each element has (arity-1) siblings
                // store it as a parent hash
                self.prefixed[j].hashes[offset] = self.prefixed[i].hash_all();
            }

            debug_assert!(
                data_len_aligned < self.prefixed.len(),
                "aligned length out of bounds in fill_layers (default hash)"
            );
            let layer_default_hash = self.prefixed[data_len_aligned].hash_all();
            for i in data_len_aligned..next_layer_ind {
                debug_assert!(
                    i < self.prefixed.len(),
                    "index out of bounds in fill_layers (default fill loop)"
                );
                let offset = i & (Self::ARITY - 1); // index modulo ARITY
                let (j, _) = self.parent_index_and_base(i, h, start_ind);

                self.prefixed[j].hashes[offset] = layer_default_hash;
            }

            let d = next_layer_ind - start_ind;
            // move on to the upper layer
            start_ind = next_layer_ind;
            next_layer_ind += d >> Self::ARITY.trailing_zeros();
            data_len_aligned = start_ind + Self::align_len(data_len_aligned, Self::ARITY);
            data_len_aligned = core::cmp::min(data_len_aligned, next_layer_ind);
        }

        self.root = self
            .prefixed
            .iter()
            .last()
            .expect("prefixed buffer is not empty. qed")
            .hash_all();
    }

    pub(crate) fn pad_leaves(&mut self, from_index: usize) {
        let default_hash = Prefixed::<H>::default_hash();
        let default_hashes = [default_hash; ARITY];

        let start_aligned_prefixed_index = Self::align_len(from_index, Self::ARITY);
        let partial_to_index = Self::ARITY * start_aligned_prefixed_index;
        // pad first partial prefixed hashes in the base layer
        for i in from_index..partial_to_index {
            let (index, offset) = location_in_prefixed(i, ARITY);
            self.prefixed[index].hashes[offset] = default_hash;
        }
        // pad the rest of hashes in the base layer
        for i in start_aligned_prefixed_index..self.base_layer_size() {
            self.prefixed[i].hashes = default_hashes;
        }
    }

    #[inline]
    fn parent_index_and_base(
        &self,
        index: usize,
        layer: usize,
        layer_base: usize,
    ) -> (usize, usize) {
        let curr_layer_len = layer_size(ARITY, self.height, layer);
        let parent_layer_base = layer_base + curr_layer_len;
        let parent_index =
            parent_layer_base + ((index - layer_base) >> Self::ARITY.trailing_zeros());

        (parent_index, parent_layer_base)
    }

    // replace_inner removed (mutation no longer supported)

    pub fn num_of_leaves(&self) -> usize {
        self.num_of_leaves
    }

    pub fn is_full(&self) -> bool {
        self.num_of_leaves == Self::ARITY * self.base_layer_size()
    }
}

impl<H: HashT> BaseTree<H> {
    /// generates proof at given index on base layer
    /// panics if index is out of bound
    #[allow(dead_code)]
    pub fn generate_proof(&self, index: usize) -> Proof<H> {
        let mut proof = Proof::<H>::from_root(self.root());
        let mut layer_base = 0;
        let mut j = index / Self::ARITY;
        let mut offset = index & (Self::ARITY - 1); // index modulo ARITY (power of 2)

        for layer in 0..self.height {
            proof.push(offset, self.prefixed[j]);

            offset = j & (Self::ARITY - 1); // index modulo ARITY
            (j, layer_base) = self.parent_index_and_base(j, layer, layer_base);
        }
        proof
    }

    pub fn root(&self) -> H::Output {
        self.root
    }

    // replace() removed: mutation API no longer part of MerkleTreeTrait

    // replace_leaf() removed: mutation API no longer part of MerkleTreeTrait
}

impl<H: HashT> Debug for BaseTree<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
        writeln!(f, "[root]:   {:?}", self.root())?;
        writeln!(f, "[arity]:   {}", Self::ARITY)?;
        writeln!(f, "[height]:          {}", self.height)?;
        writeln!(f, "[num of prefixed]: {}", self.prefixed.len())?;
        writeln!(
            f,
            "[total bytes]:     {}",
            size_of::<H::Output>() + size_of::<Prefixed<H>>() * self.prefixed.len()
        )?;
        writeln!(f, "[hash output len]: {} bytes", size_of::<H::Output>())?;
        write!(f, "{:?}", self.prefixed)
    }
}
