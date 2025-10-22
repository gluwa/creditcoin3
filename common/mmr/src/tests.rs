use super::*; // No change; please provide the current file contents with line numbers if further edits are required.

use crate::utils::partition_by_arity;
use crate::BaseTree;
use crate::HashT;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

#[cfg(feature = "par_mmr")]
use rayon::prelude::*;

#[derive(Debug)]
struct StdHash;

#[derive(Hash, Clone, Copy, Default, PartialEq, Debug)]
pub struct Wrapped8([u8; 8]);

impl From<u8> for Wrapped8 {
    fn from(n: u8) -> Self {
        let mut arr = [0u8; 8];
        arr[0] = n;
        Self(arr)
    }
}

impl HashT for StdHash {
    type Output = Wrapped8;

    fn hash(input: &[u8]) -> Self::Output {
        let mut s = DefaultHasher::new();
        input.hash(&mut s);
        Wrapped8(s.finish().to_ne_bytes())
    }
}

/// A Merkle Mountain Range (MMR) structure composed of multiple base trees.
///
/// NOTE: This struct is only used in tests and is not part of the public API.
struct Mmr<H: HashT> {
    base_trees: Vec<BaseTree<H>>,
    summit_tree: BaseTree<H>,
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

impl<H: HashT> Mmr<H> {
    fn root(&self) -> H::Output {
        self.summit_tree.root()
    }

    fn generate_proof(&self, index: usize) -> Proof<H> {
        let (base_index, inner_index) = self.base_and_inner_indexes_for(index);

        self.base_trees[base_index]
            .generate_proof(inner_index)
            .chain(self.summit_tree.generate_proof(base_index))
    }
}

fn create_input(from: u32, to: u32) -> Vec<Vec<u8>> {
    (from..to)
        .map(|i| (0..(i % 17)).map(|i| i as u8).collect::<Vec<_>>())
        .collect::<Vec<_>>()
}

#[test]
fn empty_base_tree_test() {
    let v = Vec::<&[u8]>::default();
    let tree = BaseTree::<StdHash>::from(&v[..]);
    assert_eq!(tree.root(), Default::default());
}

#[test]
fn single_input_base_tree_test() {
    let input = [[42u8, 43u8].as_slice()];
    let tree = BaseTree::<StdHash>::from(input.as_slice());
    let proof = tree.generate_proof(0);
    assert!(proof.validate([42u8, 43u8].as_slice()));
}

#[test]
fn base_tree_basic_test() {
    let input = [
        [1u8],
        [3u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [4u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [5u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
        [3u8],
        [4u8],
        [5u8],
        [6u8],
        [7u8],
        [1u8],
        [2u8],
    ];
    let tree =
        BaseTree::<StdHash>::from(&input.iter().map(|d| d.as_slice()).collect::<Vec<_>>()[..]);
    for (i, d) in input.iter().enumerate() {
        let proof = tree.generate_proof(i);
        assert!(proof.validate(d));
    }
}

#[test]
fn empty_mmr_test() {
    let v = Vec::<&[u8]>::default();
    let mmr = Mmr::<StdHash>::from(&v[..]);
    assert_eq!(mmr.root(), Default::default());
}

#[test]
fn single_input_mmr_test() {
    let input = [[42u8, 43u8].as_slice()];
    let mmr = Mmr::<StdHash>::from(input.as_slice());
    let proof = mmr.generate_proof(0);
    assert!(proof.validate([42u8, 43u8].as_slice()));
}

#[test]
fn basic_mmr_test() {
    let input = create_input(0, 123456);
    let mmr = Mmr::<StdHash>::from(&input[..]);
    for (i, d) in input.iter().enumerate() {
        let proof = mmr.generate_proof(i);
        assert!(proof.validate(d));
    }
}

#[test]
fn mmr_in_loop_test() {
    let string = vec![42u8; 1000];
    let input = vec![string.as_slice(); 300];
    for _ in 0..20_000 {
        let _ = Mmr::<StdHash>::from(&input[..]);
    }
}

#[test]
fn fail_all_mmr_test() {
    let input = (0..12345u32)
        .map(|i| (0..=(i % 42) + 1).map(|i| i as u8).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    for (i, _) in input.iter().enumerate() {
        let proof = mmr.generate_proof(i);
        if proof.validate(&vec![][..]) {
            panic!("malvalidated empty at index: {i}, input: {:?}", input[i])
        }
        if proof.validate(&vec![0u8][..]) {
            panic!("malvalidated empty at index: {i}, input: {:?}", input[i])
        }
    }
}

#[test]
fn mmr_from_big_input() {
    let input = (0..40_000_000u32)
        .map(|i| (0..=(i % 42) + 1).map(|i| i as u8).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    let proof = mmr.generate_proof(
        input
            .iter()
            .enumerate()
            .next_back()
            .map(|(i, _)| i)
            .unwrap(),
    );
    assert!(proof.validate(input.iter().next_back().unwrap()));
}

#[test]
fn mmr_from_long_input() {
    let input = (0..10_000u32)
        .map(|_| (0..100_000).map(|i| i as u8).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    let proof = mmr.generate_proof(
        input
            .iter()
            .enumerate()
            .next_back()
            .map(|(i, _)| i)
            .unwrap(),
    );
    assert!(proof.validate(input.iter().next_back().unwrap()));
}

// Removed deprecated replace_in_mmr_test: mutation API no longer part of MerkleTreeTrait
#[test]
fn same_path_offsets_for_different_indices_test() {
    let input = (0..7u8).map(|i| vec![i]).collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    let proof_offsets1 = mmr
        .generate_proof(4)
        .path()
        .iter()
        .map(|item| item.offset())
        .collect::<Vec<_>>();

    let input = (0..35u8).map(|i| vec![i]).collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    let proof_offsets2 = mmr
        .generate_proof(32)
        .path()
        .iter()
        .map(|item| item.offset())
        .collect::<Vec<_>>();

    assert_eq!(proof_offsets1, proof_offsets2);
}
