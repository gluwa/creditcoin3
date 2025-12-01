# Common Utils

A utility crate for Creditcoin3 providing essential functionality for blockchain operations, type conversions, and cryptographic operations.

## Features

- **Block Item Traits**: Interfaces for blockchain items with unique identifiers
- **Starknet Integration**: Pedersen hash implementation for Merkle structures
- **Type Conversions**: Utilities for converting between types and parsing
- **JSON Serialization**: File-based JSON serialization traits (std only)

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
utils = { path = "../common/utils" }
```

### Basic Usage

```rust
use utils::{BlockItemIdentifier, Felt, felts_from_bytes};

// Create a block item identifier
let id = BlockItemIdentifier::new(100, 5);
println!("Block: {}, Index: {}", id.block_number(), id.index());

// Work with Starknet Felts
let felt = Felt::from(42u64);

// Convert bytes to Felts and back
let data = b"Hello, Creditcoin3!";
let felts = felts_from_bytes(data);
let bytes = felts_to_bytes(&felts, Some(data.len()));
assert_eq!(data, &bytes[..]);
```

### Parsing Utilities

```rust
use utils::{try_parse_u64, try_parse_felt};

// Parse numbers that could be decimal or hex
let decimal_str = "123";
let hex_str = "0x7B";  // Same value in hex

let val1 = try_parse_u64(decimal_str)?; // 123
let val2 = try_parse_u64(hex_str)?;     // 123
assert_eq!(val1, val2);

// Parse Felt from decimal or hex
let felt1 = try_parse_felt("12345")?;
let felt2 = try_parse_felt("0x3039")?; // Same value

// Parse negative Felt values
let negative_felt = try_parse_felt("-42")?;
```

### Block Items

```rust
use utils::{BlockItem, BlockItemIdentifier};

#[derive(Debug)]
struct Transaction {
    id: BlockItemIdentifier,
    data: Vec<u8>,
}

impl BlockItem for Transaction {
    fn id(&self) -> &BlockItemIdentifier {
        &self.id
    }

    fn payload_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn tx_type(&self) -> Option<u8> {
        Some(1) // Transaction type
    }
}

let tx = Transaction {
    id: BlockItemIdentifier::new(100, 0),
    data: vec![1, 2, 3, 4],
};

let bytes = tx.to_bytes(); // Includes ID + payload
```

### Merkle Trees with Starknet Pedersen Hash

```rust
use utils::{StarknetPedersenMerkleTree, pedersen_array};
use merkle::traits::MerkleTreeTrait;

// Create a Merkle tree with Starknet Pedersen hash
let data = vec![b"leaf1", b"leaf2", b"leaf3"];
let tree = StarknetPedersenMerkleTree::from(data.as_slice());

// Generate proof for first leaf
let proof = tree.generate_proof(0);

// Use pedersen array function directly
let felts = [Felt::from(1u64), Felt::from(2u64)];
let hash = pedersen_array(&felts);
```

### JSON Serialization (std feature only)

```rust
#[cfg(feature = "std")]
use utils::JsonSerializable;

#[cfg(feature = "std")]
impl JsonSerializable for BlockItemIdentifier {}

#[cfg(feature = "std")]
fn save_and_load_example() -> anyhow::Result<()> {
    let id = BlockItemIdentifier::new(42, 100);

    // Save to file
    id.to_file("block_item.json")?;

    // Load from file
    let loaded_id = BlockItemIdentifier::try_from_file("block_item.json")?;

    assert_eq!(id, loaded_id);
    Ok(())
}
```

## Core Functionality

### Constants


### Functions

- `felts_from_bytes(bytes: &[u8]) -> Vec<Felt>`: Convert bytes to Felts using 31-byte chunks
- `felts_to_bytes(felts: &[Felt], source_bytes_len: Option<usize>) -> Vec<u8>`: Convert Felts back to bytes
- `try_parse_u64(s: &str) -> Result<u64, ParseIntError>`: Parse u64 from decimal or hex string
- `try_parse_usize(s: &str) -> Result<usize, ParseIntError>`: Parse usize from decimal or hex string
- `try_parse_felt(s: &str) -> Result<Felt, FromStrError>`: Parse Felt from decimal or hex string
- `pedersen_array<T: AsRef<Felt>>(felts: &[T]) -> Felt`: Hash array of Felts using Pedersen hash

### Types

- `Felt`: Starknet field element type (alias for `starknet_crypto::Felt`)
- `BlockItemIdentifier`: Unique identifier for items within a block
- `StarknetPedersenMerkleTree`: Merkle tree using Starknet Pedersen hash
- `StarknetPedersenMerkleProof`: Merkle proof using Starknet Pedersen hash

### Traits

- `BlockItem`: Interface for items that can be stored in a block
- `JsonSerializable`: Interface for JSON file serialization (std only)

## Features

### Default Features

- `std`: Enables standard library support including file I/O and JSON serialization

### no_std Support

This crate supports `no_std` environments when compiled without the `std` feature:

```toml
[dependencies]
utils = { path = "../common/utils", default-features = false }
```

In `no_std` mode:
- JSON serialization is not available
- File I/O operations are not available
- All core utilities and type conversions remain available

## Testing

Run the test suite:

```bash
cargo test
```

For no_std testing:

```bash
cargo test --no-default-features
```

## License

Licensed under the same terms as the parent Creditcoin3 project.
