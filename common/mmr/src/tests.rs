use super::*;
use crate::traits::{MerkleTreeTrait, ProofValidator};
use crate::HashT;
use crate::{BaseTree, Mmr};
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

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
fn base_tree_claim_index_test() {
    let input = create_input(0, 123456);
    let tree =
        BaseTree::<StdHash>::from(&input.iter().map(|d| d.as_slice()).collect::<Vec<_>>()[..]);
    for (i, _) in input.iter().enumerate() {
        let proof = tree.generate_proof(i);
        assert_eq!(proof.claim_index(), i);
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
    let proof = mmr.generate_proof(input.iter().enumerate().last().map(|(i, _)| i).unwrap());
    assert!(proof.validate(input.iter().last().unwrap()));
}

#[test]
fn mmr_from_long_input() {
    let input = (0..10_000u32)
        .map(|_| (0..100_000).map(|i| i as u8).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mmr = Mmr::<StdHash>::from(&input[..]);
    let proof = mmr.generate_proof(input.iter().enumerate().last().map(|(i, _)| i).unwrap());
    assert!(proof.validate(input.iter().last().unwrap()));
}

#[test]
fn replace_in_mmr_test() {
    let string = vec![42u8; 10];
    let input = vec![string.as_slice(); 65];
    let mut mmr1 = Mmr::<StdHash>::from(&input[..]);
    for i in 0..mmr1.num_of_leaves() {
        mmr1.replace(i, &[77u8]);
    }

    let root1 = mmr1.root();

    let replacement = vec![77u8; 1];
    let input = vec![replacement.as_slice(); 65];
    let mmr2 = Mmr::<StdHash>::from(&input[..]);
    let root2 = mmr2.root();

    assert_eq!(root1, root2);
}

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
