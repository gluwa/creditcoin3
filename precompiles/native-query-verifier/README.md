# Native Query Verifier Precompile

A high-performance native precompile for Creditcoin3 that provides efficient verification of blockchain queries using Merkle proofs and continuity chains.

## Overview

This precompile enables smart contracts to verify blockchain data from external chains (like Ethereum) directly within the Creditcoin3 runtime at native speed, without requiring external proof systems or slow interpreted contract execution.

## Features

- **Native Speed Verification**: Runs at compiled Rust speed rather than EVM interpretation
- **Merkle Proof Verification**: Validates transaction inclusion in blocks using Merkle trees
- **Continuity Chain Validation**: Verifies block attestation chains for data continuity
- **Data Extraction**: Extracts specific data segments from verified transactions
- **Gas Efficient**: Optimized gas costs for verification operations

## Architecture

```
User Query Request
        ↓
[Creditcoin Transaction]
        ↓
[Runtime Precompile Call]
        ↓
┌─────────────────────────────────────┐
│   Native Precompile Functions       │
├─────────────────────────────────────┤
│ verify_query()                      │
│    - Merkle proof verification      │
│    - Continuity chain validation    │
│    - Data extraction                │
└─────────────────────────────────────┘
        ↓
[Validators Consensus]
        ↓
[Block Inclusion]
        ↓
Result Available (1 block time)
```

## Precompile Address

The precompile is accessible at address `0x0FD2` (4050 in decimal).

## Interface

### Solidity Interface

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title Native Query Verifier Precompile Interface
/// @notice Interface for the native precompile at address 0x0BEA
interface INativeQueryVerifier {
    /// Query structure defining what data to retrieve
    struct Query {
        uint64 chain_id;      // Chain identifier (e.g., 1 for Ethereum)
        uint64 height;        // Block height
        uint64 index;         // Transaction index in block
        LayoutSegment[] layout_segments;  // Data segments to extract
    }

    /// Layout segment defining data location
    struct LayoutSegment {
        uint64 offset;        // Byte offset in transaction
        uint64 size;          // Number of bytes to extract
    }

    /// Merkle proof for transaction inclusion
    struct MerkleProof {
        bytes32 root;         // Merkle root hash
        bytes32[] siblings;   // Sibling hashes for proof path
    }

    /// Continuity chain for block attestations
    struct ContinuityChain {
        uint64[] block_numbers;  // Block numbers in chain
        bytes32[] digests;       // Block digests (hashes)
    }

    /// Result of verification
    struct QueryVerificationResult {
        uint8 status;            // 0=Success, 1=MerkleInvalid, 2=ContinuityInvalid, 3=DataError
        ResultSegment[] result_segments;  // Extracted data
    }

    /// Extracted data segment
    struct ResultSegment {
        uint64 offset;        // Offset in transaction
        bytes32 bytes;        // Extracted bytes
    }

    /// Verify a blockchain query
    /// @param query The query specification
    /// @param tx_data Raw transaction data
    /// @param merkle_proof Merkle proof for transaction inclusion
    /// @param continuity_chain Block attestation chain
    /// @return result Verification result with extracted data
    function verifyQuery(
        Query calldata query,
        bytes calldata tx_data,
        MerkleProof calldata merkle_proof,
        ContinuityChain calldata continuity_chain
    ) external view returns (QueryVerificationResult memory result);
}
```

### Usage Example

```solidity
contract MyContract {
    INativeQueryVerifier constant VERIFIER = INativeQueryVerifier(0x0000000000000000000000000000000000000BEA);

    function verifyEthereumTransaction(
        bytes calldata txData,
        bytes32 merkleRoot,
        bytes32[] calldata siblings,
        uint64[] calldata blockNumbers,
        bytes32[] calldata digests
    ) external view returns (bool) {
        // Define what data to extract (e.g., ERC20 transfer)
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](2);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address

        // Create query
        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: 1,        // Ethereum mainnet
            height: 18000000,
            index: 42,
            layout_segments: segments
        });

        // Create Merkle proof
        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        // Create continuity chain
        INativeQueryVerifier.ContinuityChain memory continuity = INativeQueryVerifier.ContinuityChain({
            block_numbers: blockNumbers,
            digests: digests
        });

        // Verify query
        INativeQueryVerifier.QueryVerificationResult memory result = VERIFIER.verifyQuery(
            query,
            txData,
            proof,
            continuity
        );

        return result.status == 0;  // 0 = Success
    }
}
```

## Gas Costs

The precompile uses the following gas cost model (aligned with standard Ethereum precompiles):

| Operation | Gas Cost | Description | Comparison |
|-----------|----------|-------------|------------|
| Base | 35,000 | Base overhead for entering precompile | ~12x ecrecover (3,000) |
| Per TX byte | 16 | Per byte of transaction data | Matches EVM calldata cost |
| Per sibling | 3,000 | Per Merkle sibling hash verification | Equal to ecrecover |
| Per continuity block | 5,000 | Per block in continuity chain | ~2x SLOAD |
| Storage lookup | 2,600 | Each attestation/checkpoint read | Matches cold SLOAD |
| Merkle verification | 100,000 weight | Fixed cost for Merkle tree traversal |
| Continuity verification | 50,000 weight | Fixed cost for continuity validation |

## Status Codes

| Code | Name | Description |
|------|------|-------------|
| 0 | Success | Verification successful, data extracted |
| 1 | MerkleProofInvalid | Merkle proof verification failed |
| 2 | ContinuityChainInvalid | Continuity chain validation failed |
| 3 | DataExtractionError | Error extracting data from transaction |

## Implementation Status

### ✅ Completed

- Precompile structure and interface
- Gas cost accounting
- Input validation
- Error handling
- Test framework

### 🚧 TODO - Fill in Core Logic

The following functions need implementation:

#### 1. `verify_merkle_proof()` (Line ~244)

Implement Merkle tree verification:
- Hash the transaction data (Keccak256 or Pedersen)
- Traverse the Merkle tree using sibling hashes
- Verify computed root matches provided root

```rust
// TODO: Replace placeholder with actual implementation
// Example structure:
let mut current_hash = sp_io::hashing::keccak_256(tx_data);
for sibling in &merkle_proof.siblings {
    current_hash = compute_parent_hash(&current_hash, sibling);
}
Ok(H256::from(current_hash) == merkle_proof.root)
```

#### 2. `verify_continuity_chain()` (Line ~292)

Implement continuity chain validation:
- Verify block numbers are properly ordered
- Check each digest matches an attestation or checkpoint
- Verify the chain connects to the queried block height

```rust
// TODO: Replace placeholder with actual implementation
// Example structure:
for (block_num, digest) in block_numbers.iter().zip(digests.iter()) {
    if let Some(attestation) = Runtime::Attestations::get_attestation(chain_id, *digest) {
        verify attestation.header_number == *block_num
    } else if let Some(checkpoint) = Runtime::Checkpoints::get_checkpoint(chain_id, *digest) {
        verify checkpoint == *block_num
    } else {
        return Ok(false);
    }
}
```

#### 3. `extract_data_segments()` (Line ~368)

Implement data extraction from verified transaction:
- Validate segment offsets and sizes
- Extract bytes at specified offsets
- Convert to H256 format (pad if necessary)

```rust
// TODO: Replace placeholder with actual implementation
// Example structure:
for segment in &query.layout_segments {
    let start = segment.offset as usize;
    let end = start + segment.size as usize;

    if end > tx_data.len() {
        return Err(out_of_bounds_error);
    }

    let bytes = &tx_data[start..end];
    let mut padded = [0u8; 32];
    padded[..bytes.len()].copy_from_slice(bytes);

    result_segments.push(ResultSegment {
        offset: segment.offset,
        bytes: H256::from(padded),
    });
}
```

## Testing

Run the test suite:

```bash
cargo test -p pallet-evm-precompile-native-query-verifier
```

### Test Coverage

- ✅ Empty transaction data validation
- ✅ Empty Merkle proof validation
- ✅ Continuity chain length mismatch
- ✅ Valid inputs success case
- ✅ Gas cost calculations
- ✅ Out of bounds segment handling
- ✅ Multiple segments extraction
- ✅ Error message encoding

## Integration

### Adding to Runtime

1. Add to `runtime/src/precompiles.rs`:

```rust
use pallet_evm_precompile_native_query_verifier::NativeQueryVerifierPrecompile;

impl PrecompileSet for GluwaPrecompiles<R> {
    fn execute(&self, handle: &mut impl PrecompileHandle) -> Option<PrecompileResult> {
        match handle.code_address() {
            // ... existing precompiles ...
            a if a == hash(4050) => Some(NativeQueryVerifierPrecompile::<Runtime>::execute(handle)),
            _ => None,
        }
    }
}
```

2. Add to workspace `Cargo.toml`:

```toml
[workspace.dependencies]
pallet-evm-precompile-native-query-verifier = { path = "precompiles/native-query-verifier", default-features = false }
```

3. Add to runtime `Cargo.toml`:

```toml
[dependencies]
pallet-evm-precompile-native-query-verifier = { workspace = true }

[features]
std = [
    # ... other features ...
    "pallet-evm-precompile-native-query-verifier/std",
]
```

## Security Considerations

- **Input Validation**: All inputs are validated before processing
- **Gas Accounting**: Proper gas costs prevent DoS attacks
- **Bounded Data**: Transaction data limited to 10MB, proofs to 1KB
- **No Reentrancy**: Uses `forbid-evm-reentrancy` feature
- **Deterministic**: All operations must be deterministic for consensus

## Development

### Project Structure

```
native-query-verifier/
├── Cargo.toml          # Dependencies and features
├── README.md           # This file
└── src/
    ├── lib.rs          # Main precompile implementation
    ├── mock.rs         # Test runtime configuration
    └── tests.rs        # Unit tests
```

### Dependencies

- `precompile-utils`: Precompile macros and utilities
- `pallet-prover-primitives`: Query and result structures
- `attestor-primitives`: Chain and attestation types
- `sp-core`, `sp-io`, `sp-std`: Substrate primitives
- `frame-support`, `frame-system`: Frame system support
- `pallet-evm`: EVM pallet integration

## Contributing

When implementing the core verification logic:

1. Maintain deterministic behavior (required for consensus)
2. Add comprehensive tests for edge cases
3. Document gas cost impacts of changes
4. Ensure error messages are descriptive
5. Follow existing code style and patterns

## License

This precompile is part of the Creditcoin3 project and follows the same license.

## References

- [Creditcoin3 Documentation](../../docs/)
- [EVM Precompiles](https://www.evm.codes/precompiled)
- [Merkle Proofs](https://en.wikipedia.org/wiki/Merkle_tree)
