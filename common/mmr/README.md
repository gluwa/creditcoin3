# MMR (Merkle Mountain Range) Package

A high-performance Merkle tree implementation optimized for blockchain data verification, proof generation, and cross-chain attestation in the Creditcoin3 ecosystem.

## Overview

The `mmr` package provides a flexible and efficient binary Merkle tree implementation with support for multiple hash functions. While named "MMR" for historical reasons, this package actually implements standard binary Merkle trees rather than Merkle Mountain Ranges.

### Key Features

- **Binary Merkle Trees**: Fixed arity of 2 (binary trees) for optimal proof sizes
- **Multiple Hash Functions**: Support for Keccak256 and custom hash implementations
- **Proof Generation & Verification**: Efficient generation and verification of Merkle proofs
- **Query Proofs**: Specialized proof format for transaction verification in the Native Query Verifier precompile
- **No-std Support**: Can be used in resource-constrained environments
- **Parallel Processing**: Optional parallel tree construction with the `par_mmr` feature

## Architecture

### Core Components

#### 1. **BaseTree** (`lib.rs`)
The main Merkle tree implementation that handles:
- Tree construction from leaf data
- Root hash calculation
- Proof generation for specific leaf indices
- Tree traversal and verification logic

```rust
use mmr::{BaseTree, traits::MerkleTreeTrait};
use mmr::keccak::Keccak256;

// Create a tree from transaction data
let transactions = vec![tx1_bytes, tx2_bytes, tx3_bytes];
let tree: BaseTree<Keccak256> = BaseTree::from(&transactions[..]);

// Get the root hash
let root = tree.root();

// Generate a proof for transaction at index 1
let proof = tree.generate_proof(1);
```

#### 2. **HashT Trait** (`traits.rs`)
Defines the interface for hash functions used in the Merkle tree:
- Hashing of single values
- Hashing of concatenated values
- Domain separation support

```rust
pub trait HashT {
    type Output: Hash + Default + Copy + From<u8> + Send + Sync + PartialEq + Debug;

    fn hash(data: &[u8]) -> Self::Output;
    fn hash2(left: &Self::Output, right: &Self::Output) -> Self::Output;
}
```

#### 3. **Keccak256 Implementation** (`keccak.rs`)
Ethereum-compatible Keccak256 hash function implementation:
- Wrapper type `KeccakHash` around H256
- Full `HashT` trait implementation
- Used for Ethereum transaction and receipt Merkle trees

```rust
use mmr::keccak::{Keccak256, KeccakHash};

let hash = Keccak256::hash(b"hello world");
```

#### 4. **QueryMerkleProof** (`query_proof.rs`)
Specialized proof format for the Native Query Verifier precompile:
- Compatible with Solidity/EVM verification
- Includes sibling position information (left/right)
- Optimized for on-chain verification

```rust
pub struct QueryMerkleProof {
    pub root: H256,
    pub siblings: Vec<MerkleProofEntry>,
}

pub struct MerkleProofEntry {
    pub hash: H256,
    pub is_left: bool,  // Position information for verification
}
```

#### 5. **Proof Types** (`proof.rs`)
Standard Merkle proof representations:
- `Proof`: Internal proof representation
- `SerializedProof`: Serialization-friendly format
- Conversion utilities between formats

## Usage Examples

### Creating a Merkle Tree from Transactions

```rust
use mmr::{BaseTree, traits::MerkleTreeTrait};
use mmr::keccak::Keccak256;

fn create_transaction_tree(transactions: Vec<Vec<u8>>) -> BaseTree<Keccak256> {
    // Convert transactions to byte slices
    let tx_refs: Vec<&[u8]> = transactions.iter().map(|tx| tx.as_slice()).collect();

    // Create the Merkle tree
    BaseTree::from(&tx_refs[..])
}
```

### Generating a Query Proof for Verification

```rust
use mmr::query_proof::QueryMerkleProof;
use mmr::traits::MerkleTreeTrait;

fn generate_query_proof(tree: &BaseTree<Keccak256>, tx_index: usize) -> QueryMerkleProof {
    // Generate standard proof
    let proof = tree.generate_proof(tx_index);

    // Convert to query proof format
    QueryMerkleProof::from_proof(proof, tx_index)
}
```

### Verifying a Merkle Proof

```rust
use mmr::query_proof::QueryMerkleProof;
use sp_core::H256;

fn verify_transaction_inclusion(
    tx_hash: H256,
    proof: &QueryMerkleProof,
) -> bool {
    // Start with the transaction hash
    let mut current = tx_hash;

    // Apply each sibling hash
    for sibling in &proof.siblings {
        if sibling.is_left {
            // Sibling is on the left, current node is on the right
            current = keccak256(&[sibling.hash.as_bytes(), current.as_bytes()].concat());
        } else {
            // Sibling is on the right, current node is on the left
            current = keccak256(&[current.as_bytes(), sibling.hash.as_bytes()].concat());
        }
    }

    // Check if we reach the root
    current == proof.root
}
```

### Working with Block Headers

```rust
use mmr::{BaseTree, traits::MerkleTreeTrait};
use mmr::keccak::Keccak256;

fn create_receipts_tree(receipts: Vec<Vec<u8>>) -> H256 {
    let receipt_refs: Vec<&[u8]> = receipts.iter().map(|r| r.as_slice()).collect();
    let tree: BaseTree<Keccak256> = BaseTree::from(&receipt_refs[..]);

    // Return the receipts root for the block header
    H256::from(tree.root().0)
}
```

## Integration with Native Query Verifier

The MMR package is a critical component of the Native Query Verifier precompile, providing:

1. **Transaction Proof Generation**: Creates Merkle proofs for transactions in a block
2. **Receipt Proof Generation**: Creates Merkle proofs for transaction receipts
3. **Efficient Verification**: Optimized proof format for on-chain verification
4. **EVM Compatibility**: Proofs can be verified in Solidity smart contracts

### Example: Preparing Data for the Precompile

```rust
use mmr::query_proof::QueryMerkleProof;
use attestor_primitives::query::Query;

fn prepare_verification_data(
    tx_data: Vec<u8>,
    tx_index: usize,
    block_transactions: Vec<Vec<u8>>,
) -> (Vec<u8>, QueryMerkleProof) {
    // Create Merkle tree of all transactions
    let tx_refs: Vec<&[u8]> = block_transactions.iter().map(|tx| tx.as_slice()).collect();
    let tree = BaseTree::<Keccak256>::from(&tx_refs[..]);

    // Generate proof for the specific transaction
    let proof = tree.generate_proof(tx_index);
    let query_proof = QueryMerkleProof::from_proof(proof, tx_index);

    (tx_data, query_proof)
}
```

## Features

### `std` (default)
Enables standard library support and all features:
- Parallel processing
- Full error messages
- Additional utility functions

### `par_mmr`
Enables parallel tree construction using Rayon:
- Faster tree building for large datasets
- Automatically enabled with `std`

### `no_std`
Disable by using `default-features = false`:
```toml
[dependencies]
mmr = { version = "3.66.0", default-features = false }
```

## Performance Considerations

### Tree Construction
- **Sequential**: O(n log n) for n leaves
- **Parallel** (with `par_mmr`): O(n) with sufficient cores
- Memory: O(n) for storing intermediate hashes

### Proof Generation
- Time: O(log n) for n leaves
- Proof size: O(log n) hashes

### Verification
- Time: O(log n) hash operations
- Space: O(1) temporary storage

## Security Considerations

1. **Hash Function Choice**: Use Keccak256 for Ethereum compatibility
2. **Empty Tree Handling**: Empty trees are not supported; ensure at least one leaf
3. **Proof Validation**: Always verify proof structure before verification
4. **Index Bounds**: Validate leaf indices are within tree bounds

## Testing

The package includes comprehensive tests in `tests.rs`:
- Tree construction with various sizes
- Proof generation and verification
- Edge cases (single leaf, power of 2, non-power of 2)
- Hash function implementations
- Serialization/deserialization

Run tests:
```bash
cargo test -p mmr
```

With all features:
```bash
cargo test -p mmr --all-features
```

## Dependencies

- `sp-core`: Core Substrate types (H256)
- `sp-io`: Hashing functions (keccak256)
- `parity-scale-codec`: SCALE codec for serialization
- `scale-info`: Type information for runtime
- `precompile-utils`: Utilities for precompile development
- `rayon` (optional): Parallel processing support

## License

This package is part of the Creditcoin3 project and follows the project's licensing terms.

## Contributing

When contributing to this package:
1. Maintain backward compatibility with existing proof formats
2. Ensure no-std compatibility is preserved
3. Add tests for new functionality
4. Update benchmarks if performance characteristics change
5. Document any changes to the proof format

## Related Packages

- `attestor-primitives`: Core types for attestation system
- `native-query-verifier`: Precompile using MMR proofs
- `query-cli`: Command-line tool for generating and verifying proofs
