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

#[cfg(feature = "par_mmr")]
use rayon::prelude::*;

use crate::prefixed::Prefixed;
use crate::proof::Proof;
use crate::traits::{HashT, MerkleTreeTrait};
use crate::utils::{
    height, layer_size, location_in_prefixed, num_of_prefixed_for_input, partition_by_arity,
};

pub const ARITY: Arity = Arity::Two;

/// leaves will be prepended with this value prior to hashing
pub const LEAF_HASH_PREPEND_VALUE: u8 = 0;
/// inner nodes will be prepended with this value prior to hashing
pub const INNER_HASH_PREPEND_VALUE: u8 = 1;

#[derive(Debug)]
pub enum Error {
    Append,
}

#[derive(Debug, Clone, Copy)]
pub enum Arity {
    Two = 2,
    Four = 4,
    Eight = 8,
    Sixteen = 16,
}

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
    const ARITY: usize = ARITY as usize;

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
                let offset = i & (Self::ARITY - 1); // index modulo ARITY
                let (j, _) = self.parent_index_and_base(i, h, start_ind);

                // hash concatenated siblings from the contiguous memory
                // each element has (arity-1) siblings
                // store it as a parent hash
                self.prefixed[j].hashes[offset] = self.prefixed[i].hash_all();
            }

            let layer_default_hash = self.prefixed[data_len_aligned].hash_all();
            for i in data_len_aligned..next_layer_ind {
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
        let default_hashes = [default_hash; ARITY as usize];

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

    fn replace_inner(&mut self, index: usize) {
        if self.height == 0 {
            self.root = self.prefixed[0].hashes[0];
            return;
        }
        let mut layer_base = 0;
        let mut j = index / Self::ARITY;

        // start from the base layer and propagate the new hashes upwords
        for layer in 0..self.height - 1 {
            let parent_hashed = self.prefixed[j].hash_all();

            let offset = j & (Self::ARITY - 1); // index modulo ARITY
            (j, layer_base) = self.parent_index_and_base(j, layer, layer_base);

            self.prefixed[j].hashes[offset] = parent_hashed;
        }
        self.root = self
            .prefixed
            .iter()
            .last()
            .expect("prefixed buffer is not empty. qed")
            .hash_all();
    }

    pub fn num_of_leaves(&self) -> usize {
        self.num_of_leaves
    }

    pub fn is_full(&self) -> bool {
        self.num_of_leaves == Self::ARITY * self.base_layer_size()
    }
}

impl<H: HashT> MerkleTreeTrait<H> for BaseTree<H> {
    /// generates proof at given index on base layer
    /// panics if index is out of bound
    fn generate_proof(&self, index: usize) -> Proof<H> {
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

    fn root(&self) -> H::Output {
        self.root
    }
    /// replace an element at index with input
    /// panics if index is out of leaf layer bound
    fn replace(&mut self, index: usize, input: &[u8]) {
        let prefixed_len = input.len() + 1;
        let mut prefixed = vec![LEAF_HASH_PREPEND_VALUE; prefixed_len];
        prefixed[1..prefixed_len].copy_from_slice(input);

        let (prefixed_index, offset) = location_in_prefixed(index, ARITY);

        self.prefixed[prefixed_index].hashes[offset] = H::hash(&prefixed[..]);
        self.replace_inner(index);
    }

    fn replace_leaf(&mut self, index: usize, leaf: H::Output) {
        let (prefixed_index, offset) = location_in_prefixed(index, ARITY);
        self.prefixed[prefixed_index].hashes[offset] = leaf;

        self.replace_inner(index);
    }
    fn leaves(&self) -> &[Prefixed<H>] {
        &self.prefixed[..self.base_layer_size()]
    }
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

pub struct Mmr<H: HashT> {
    base_trees: Vec<BaseTree<H>>,
    summit_tree: BaseTree<H>,
    num_of_leaves: usize,
}

#[cfg(feature = "par_mmr")]
impl<H, T> From<&[T]> for Mmr<H>
where
    H: HashT,
    T: AsRef<[u8]> + Deref<Target = [u8]> + Send + Sync + Debug,
{
    fn from(input: &[T]) -> Self {
        let len = input.len();
        let partition_offsets = partition_by_arity(len, ARITY);

        let base_trees = partition_offsets
            .par_windows(2)
            .map(|w| BaseTree::from(&input[w[0]..w[1]]))
            .collect::<Vec<_>>();

        let summit_tree =
            BaseTree::from_leaves(&base_trees.iter().map(BaseTree::root).collect::<Vec<_>>()[..]);

        Self {
            base_trees,
            summit_tree,
            num_of_leaves: len,
        }
    }
}

#[cfg(not(feature = "par_mmr"))]
impl<H, T> From<&[T]> for Mmr<H>
where
    H: HashT,
    T: AsRef<[u8]> + Deref<Target = [u8]> + Debug,
{
    fn from(input: &[T]) -> Self {
        let len = input.len();
        let partition_offsets = partition_by_arity(len, ARITY);

        let base_trees = partition_offsets
            .windows(2)
            .map(|w| BaseTree::from(&input[w[0]..w[1]]))
            .collect::<Vec<_>>();

        let summit_tree =
            BaseTree::from_leaves(&base_trees.iter().map(BaseTree::root).collect::<Vec<_>>()[..]);

        Self {
            base_trees,
            summit_tree,
            num_of_leaves: len,
        }
    }
}

impl<H: HashT> Mmr<H> {
    pub fn num_of_leaves(&self) -> usize {
        self.num_of_leaves
    }

    fn base_and_inner_indexes_for(&self, index: usize) -> (usize, usize) {
        let mut accrue_len = 0;
        let mut i = 0;
        // find the peak corresponding to the index
        while accrue_len + self.base_trees[i].num_of_leaves() <= index {
            accrue_len += self.base_trees[i].num_of_leaves();
            i += 1;
        }
        (i, index - accrue_len)
    }
}

impl<H: HashT> MerkleTreeTrait<H> for Mmr<H> {
    fn root(&self) -> H::Output {
        self.summit_tree.root()
    }

    fn generate_proof(&self, index: usize) -> Proof<H> {
        let (base_index, inner_index) = self.base_and_inner_indexes_for(index);

        self.base_trees[base_index]
            .generate_proof(inner_index)
            .chain(self.summit_tree.generate_proof(base_index))
    }

    fn replace(&mut self, index: usize, input: &[u8]) {
        let (base_index, inner_index) = self.base_and_inner_indexes_for(index);

        self.base_trees[base_index].replace(inner_index, input);
        self.summit_tree
            .replace_leaf(base_index, self.base_trees[base_index].root());
    }
    fn replace_leaf(&mut self, index: usize, leaf: H::Output) {
        let (base_index, inner_index) = self.base_and_inner_indexes_for(index);

        self.base_trees[base_index].replace_leaf(inner_index, leaf);
        self.summit_tree
            .replace_leaf(base_index, self.base_trees[base_index].root());
    }
    fn leaves(&self) -> &[Prefixed<H>] {
        unimplemented!();
    }
}
