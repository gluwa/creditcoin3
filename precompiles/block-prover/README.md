# Native Query Verifier Precompile

A high-performance native precompile for Creditcoin3 that provides efficient verification of blockchain queries using Merkle proofs and continuity chains.

## Overview

This precompile enables smart contracts to verify blockchain data from external chains (like Ethereum) directly within the Creditcoin3 runtime at native speed, without requiring external proof systems or slow interpreted contract execution.

## Features

- **Native Speed Verification**: Runs at compiled Rust speed rather than EVM interpretation
- **Merkle Proof Verification**: Validates transaction inclusion in blocks using Merkle trees
- **Continuity Chain Validation**: Verifies block attestation chains for data continuity
- **Query Block Digest Verification**: Validates block digests using previous block's digest to prevent fake roots
- **Data Extraction**: Extracts specific data segments from verified transactions
- **Gas Efficient**: Optimized gas costs for verification operations
- **Batch Verification**: Supports up to 10 queries with shared continuity proof for significant gas savings
- **View Functions**: Read-only functions (`verifyQueryView`, `verifyBatchQueriesView`) that don't emit events
- **Event Emission**: Non-view functions emit detailed events for indexers and monitoring
- **Optimized Block Lookups**: Efficient O(1) lookup for sequential blocks, O(log n) binary search fallback

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
│ verifyQuery() / verifyQueryView()   │
│    - Merkle proof verification      │
│    - Query block digest validation  │
│    - Continuity chain validation    │
│    - Data extraction                │
│    - Event emission (non-view only) │
│                                      │
│ verifyBatchQueries() /              │
│ verifyBatchQueriesView()            │
│    - Shared continuity verification │
│    - Per-query Merkle verification  │
│    - Batch event emission           │
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

See the [INativeQueryVerifier interface](https://github.com/gluwa/creditcoin3-next/blob/main/precompiles/metadata/sol/INativeQueryVerifier.sol) for the complete Solidity interface definition.

### Usage Example

See [ExampleUsage.sol](https://github.com/gluwa/creditcoin3-next/blob/main/precompiles/native-query-verifier/ExampleUsage.sol) for complete usage examples including:
- Single query verification
- Batch query verification
- Cross-chain bridge implementation
- Best practices for storing verification results

## Key Optimizations

### 1. Batch Verification with Shared Continuity Proof

For batch queries, the precompile uses an optimized verification approach:
- The continuity proof chain is verified **once** upfront (shared across all queries)
- Each individual query only needs to verify:
  - Merkle proof for transaction inclusion
  - Query block digest using previous block's digest
  - Query block exists in the continuity chain
  - Merkle root matches the continuity block

This is significantly more efficient than strict verification which would validate the entire continuity chain for each query. The optimized mode is safe because the primary goal is proving **inclusion** - as long as the transaction is in the block and the block's digest is validated against the previous block and ultimately against a checkpoint, full sequential chain verification per query is not necessary.

### 2. Optimized Block Lookups

The precompile optimizes block lookups using multiple strategies:
- **Sequential blocks**: O(1) lookup using computed index (block_number = start_block + index)
- **Sorted blocks**: O(log n) binary search for non-sequential but sorted blocks
- **Unsorted blocks**: O(n) linear search fallback

This optimization reduces computational overhead when processing large continuity chains.

### 3. Query Block Digest Verification

The precompile implements a critical security check: query block digest verification. This ensures that:
- The query block's digest is computed using the previous block's digest
- Prevents attackers from sending fake Merkle roots
- Requires at least 2 blocks in the continuity chain (queryHeight-1 and queryHeight)

## Gas Costs

The precompile uses the following gas cost model (aligned with standard Ethereum precompiles):

| Operation | Gas Cost | Description | Comparison |
|-----------|----------|-------------|------------|
| Base | 21,000 | Base transaction cost | Matches Ethereum standard transaction cost |
| Per TX byte | 16 | Per byte of transaction data | Matches EVM calldata cost |
| Per sibling | 200 | Per Merkle sibling hash verification | Native efficiency vs ~166 in Solidity |
| Per continuity block | 400 | Per block verification (hash + overhead) | Covers hash computation (~48 gas) + comparisons/overhead (~350 gas) |
| Hash computation | 48 | Keccak-256 hash (72 bytes = 3 words) | 30 base + 6 per word (matches Ethereum KECCAK256) |
| Storage lookup | 2,600 | Each attestation/checkpoint read | Matches cold SLOAD |
| Merkle verification weight | 100,000 | Fixed weight for Merkle tree traversal | Converted to gas via runtime weight-to-gas mapping |
| Continuity verification weight | 50,000 | Fixed weight for continuity validation | Converted to gas via runtime weight-to-gas mapping |
| Event emission (non-view only) | ~1,000 | Per event log (3 topics × 32 bytes) | Charged only for non-view functions |

## Status Codes

| Code | Name | Description |
|------|------|-------------|
| 0 | Success | Verification successful, data extracted |
| 1 | MerkleProofInvalid | Merkle proof verification failed |
| 2 | ContinuityChainInvalid | Continuity chain validation failed |
| 3 | DataExtractionError | Error extracting data from transaction |
| 4 | MerkleRootMismatch | Merkle proof root doesn't match continuity block |

**Note**: Non-view functions (`verifyQuery`, `verifyBatchQueries`) revert on failure with descriptive error messages. View functions (`verifyQueryView`, `verifyBatchQueriesView`) return status codes in the result structure.

## Functions

### View Functions (No Events)

- **`verifyQueryView`**: Read-only verification without event emission
- **`verifyBatchQueriesView`**: Read-only batch verification without events

Use view functions when you only need verification results and don't need on-chain event logs.

### Non-View Functions (With Events)

- **`verifyQuery`**: Verification with event emission
- **`verifyBatchQueries`**: Batch verification with events

Use non-view functions when you need events for indexers, monitoring, or on-chain event listeners.

## Batch Query Verification

The precompile supports batch verification of up to 10 queries with a shared continuity proof, providing significant gas savings:

### Features

- **Shared Continuity Proof**: Verifies the continuity chain once for all queries
- **Gas Optimization**: For 5 queries with 20-block continuity, saves ~3,200 gas per additional query (shared continuity verification)
- **Individual Event Emission**: Emits `QueryVerified` or `QueryVerificationFailed` for each query (non-view only)
- **Summary Event**: Emits `BatchQueriesVerified` with aggregate statistics (non-view only)

### Usage Example

See [ExampleUsage.sol](https://github.com/gluwa/creditcoin3-next/blob/main/precompiles/native-query-verifier/ExampleUsage.sol) for batch verification examples.

### Events

When calling `verifyQuery` or `verifyBatchQueries` (non-view functions), the following events are emitted:

#### QueryVerified
```solidity
event QueryVerified(
    address indexed caller,
    bytes32 queryId,
    uint64 chainKey,
    uint64 height,
    uint8 status,
    ResultSegment[] resultSegments
);
```

#### QueryVerificationFailed
```solidity
event QueryVerificationFailed(
    address indexed caller,
    bytes32 queryId,
    uint64 chainKey,
    uint64 height,
    uint8 status,
    string reason
);
```

#### BatchQueriesVerified
```solidity
event BatchQueriesVerified(
    uint256 successful,
    uint256 failed,
    uint256 total
);
```

**Note**: View functions (`verifyQueryView`, `verifyBatchQueriesView`) do NOT emit events. This ensures read-only operations don't modify state or generate logs.

## Implementation Status

### ✅ Fully Implemented

- **Precompile Structure**: Complete interface with view and non-view functions
- **Merkle Proof Verification**: Full Keccak256 Merkle tree verification with position-aware siblings
- **Query Block Digest Verification**: Validates block digests using previous block's digest
- **Continuity Chain Validation**: Verifies chains against attestations and checkpoints
- **Data Extraction**: Extracts and validates data segments from verified transactions
- **Batch Verification**: Optimized batch processing with shared continuity proof
- **Gas Cost Accounting**: Comprehensive gas cost model for all operations
- **Input Validation**: Bounds checking and validation for all inputs
- **Error Handling**: Descriptive error messages with proper revert encoding
- **Event Emission**: Complete event system for monitoring and indexing
- **Test Coverage**: Comprehensive test suite including edge cases and gas tests

## Testing

Run the test suite:

```bash
cargo test -p pallet-evm-precompile-native-query-verifier
```

### Test Coverage

The test suite includes comprehensive coverage:

- ✅ **Single Query Verification**: Success and failure cases
- ✅ **Batch Query Verification**: Shared continuity proof optimization
- ✅ **View Functions**: Read-only verification without events
- ✅ **Merkle Proof Validation**: Invalid proofs, root mismatches
- ✅ **Continuity Chain Validation**: Broken links, missing attestations
- ✅ **Query Block Digest Verification**: Digest mismatch detection
- ✅ **Data Extraction**: Out of bounds, multiple segments
- ✅ **Edge Cases**: Empty inputs, queries at attestation/checkpoint heights
- ✅ **Gas Cost Calculations**: Accurate gas accounting
- ✅ **Event Emission**: Correct events for success and failure
- ✅ **Error Handling**: Descriptive error messages

## Integration

### Adding to Runtime

1. Add to `runtime/src/precompiles.rs`:

```rust
use pallet_evm_precompile_native_query_verifier::BlockProverPrecompile;

impl PrecompileSet for GluwaPrecompiles<R> {
    fn execute(&self, handle: &mut impl PrecompileHandle) -> Option<PrecompileResult> {
        match handle.code_address() {
            // ... existing precompiles ...
            a if a == hash(4050) => Some(BlockProverPrecompile::<Runtime>::execute(handle)),
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
├── Cargo.toml                    # Dependencies and features
├── README.md                     # This file
├── ExampleUsage.sol              # Solidity usage examples
└── src/
    ├── lib.rs                    # Main precompile implementation
    ├── verify.rs                 # Core verification logic
    ├── continuity.rs             # Continuity chain validation
    ├── mock.rs                   # Test runtime configuration
    ├── test_helpers.rs           # Test utilities and helpers
    ├── tests.rs                  # Basic unit tests
    ├── tests_view.rs             # View function tests
    ├── tests_full_coverage.rs    # Comprehensive coverage tests
    └── tests_gas_security.rs     # Gas and security tests
```

### Dependencies

- `precompile-utils`: Precompile macros and utilities
- `attestor-primitives`: Block, query, and attestation types
- `mmr`: Merkle tree and proof generation
- `sp-core`, `sp-io`, `sp-std`: Substrate primitives
- `frame-support`, `frame-system`: Frame system support
- `pallet-evm`: EVM pallet integration
- `ethabi`: ABI encoding/decoding

## Security Considerations

### Query Block Digest Verification

The precompile implements a critical security check: query block digest verification. This prevents attackers from sending fake Merkle roots by requiring:

1. At least 2 blocks in the continuity chain (queryHeight-1 and queryHeight)
2. The query block's digest is computed using: `hash_payload(queryHeight, merkleRoot, prevBlockDigest)`
3. The computed digest must match the query block's stored digest

This ensures that the Merkle root being verified actually belongs to the attested block, not just any block with a valid Merkle tree.

### Continuity Chain Validation

The continuity chain must:
- Start at `queryHeight - 1` (required for digest verification)
- End at an attestation or checkpoint height
- Have all digests validated against on-chain attestations/checkpoints
- Have properly linked digests (each block's digest uses the previous block's digest)

### Input Validation

- Transaction data: Maximum 10MB
- Merkle proof siblings: Bounded by tree depth
- Batch queries: Maximum 10 queries per batch
- Layout segments: Validated for bounds and overlap

## Contributing

When contributing to this precompile:

1. Maintain deterministic behavior (required for consensus)
2. Add comprehensive tests for edge cases
3. Document gas cost impacts of changes
4. Ensure error messages are descriptive
5. Follow existing code style and patterns
6. Update this README when adding new features
7. Ensure view functions never emit events
8. Ensure non-view functions always emit appropriate events

## License

This precompile is part of the Creditcoin3 project and follows the same license.

## References

- [Creditcoin3 Documentation](../../docs/)
- [EVM Precompiles](https://www.evm.codes/precompiled)
- [Merkle Proofs](https://en.wikipedia.org/wiki/Merkle_tree)
