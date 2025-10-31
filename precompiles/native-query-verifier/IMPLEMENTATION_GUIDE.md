# Native Query Verifier Precompile - Implementation Guide

This guide explains what needs to be implemented in the three core functions of the Native Query Verifier precompile.

## Overview

The boilerplate provides:
- ✅ Complete precompile structure and interface
- ✅ Gas cost accounting
- ✅ Input validation
- ✅ Error handling
- ✅ Test framework
- ✅ Solidity interface and examples

What you need to implement:
- 🚧 Merkle proof verification logic
- 🚧 Continuity chain validation logic
- 🚧 Data extraction logic

## Location

All implementation work is in: `creditcoin3-next/precompiles/native-query-verifier/src/lib.rs`

---

## 1. Merkle Proof Verification (`verify_merkle_proof`)

**Location:** `src/lib.rs` around line 244

**Purpose:** Verify that a transaction is included in a block using a Merkle proof.

### What You Need to Implement

```rust
fn verify_merkle_proof(
    _handle: &mut impl PrecompileHandle,
    tx_data: &[u8],
    merkle_proof: &MerkleProof,
    query: &Query,
) -> Result<bool, PrecompileFailure>
```

### Algorithm Steps

1. **Hash the transaction data**
   - Use Keccak256 (Ethereum-style) or Pedersen hash (for Cairo/StarkNet)
   - This gives you the leaf hash of the Merkle tree
   ```rust
   use sp_io::hashing::keccak_256;
   let mut current_hash = keccak_256(tx_data);
   ```

2. **Traverse the Merkle tree**
   - For each sibling hash in the proof:
     - Combine current hash with sibling (order matters!)
     - Hash the combination
     - This becomes the new current hash (moving up the tree)
   ```rust
   for sibling in &merkle_proof.siblings {
       // Determine order based on tree structure
       // Option A: current left, sibling right
       let combined = [current_hash, sibling.as_bytes()].concat();
       // Option B: sibling left, current right (depends on tree construction)
       // let combined = [sibling.as_bytes(), current_hash].concat();

       current_hash = keccak_256(&combined);
   }
   ```

3. **Verify the computed root**
   - Compare final hash with provided root
   ```rust
   Ok(H256::from(current_hash) == merkle_proof.root)
   ```

### Key Considerations

- **Hash function choice**: Must match the chain you're verifying (Ethereum = Keccak256)
- **Sibling ordering**: Left/right position matters in Merkle trees
  - You may need to track position using `query.index`
  - For index-based ordering: if bit i is 0, current is left; if 1, current is right
- **Edge cases**:
  - Empty siblings array (already validated in main function)
  - Single-transaction block (no siblings)

### Example Implementation Skeleton

```rust
fn verify_merkle_proof(
    _handle: &mut impl PrecompileHandle,
    tx_data: &[u8],
    merkle_proof: &MerkleProof,
    query: &Query,
) -> Result<bool, PrecompileFailure> {
    // 1. Hash the transaction
    let mut current_hash = sp_io::hashing::keccak_256(tx_data);
    let mut index = query.index;

    // 2. Traverse the tree
    for sibling in &merkle_proof.siblings {
        let sibling_bytes: [u8; 32] = sibling.to_fixed_bytes();

        // Determine position based on index bit
        let combined = if index & 1 == 0 {
            // Current is left, sibling is right
            [&current_hash[..], &sibling_bytes[..]].concat()
        } else {
            // Sibling is left, current is right
            [&sibling_bytes[..], &current_hash[..]].concat()
        };

        current_hash = sp_io::hashing::keccak_256(&combined);
        index >>= 1; // Move to parent level
    }

    // 3. Verify root
    let computed_root = H256::from(current_hash);
    Ok(computed_root == merkle_proof.root)
}
```

---

## 2. Continuity Chain Verification (`verify_continuity_chain`)

**Location:** `src/lib.rs` around line 292

**Purpose:** Verify that a chain of blocks is properly attested through checkpoints or attestations.

### What You Need to Implement

```rust
#[cfg(not(feature = "runtime-benchmarks"))]
fn verify_continuity_chain(
    handle: &mut impl PrecompileHandle,
    continuity_chain: &ContinuityChain,
    query: &Query,
) -> Result<bool, PrecompileFailure>
```

### Algorithm Steps

1. **Validate chain ordering**
   - Block numbers should be sequential or properly ordered
   - No gaps larger than expected attestation intervals
   ```rust
   for i in 0..continuity_chain.block_numbers.len() - 1 {
       if continuity_chain.block_numbers[i] >= continuity_chain.block_numbers[i + 1] {
           return Ok(false); // Not properly ordered
       }
   }
   ```

2. **Verify each block against attestations/checkpoints**
   - For each (block_number, digest) pair:
     - Try to find matching attestation first
     - Fall back to checkpoint if no attestation found
     - Verify the block number matches
   ```rust
   for (block_num, digest) in block_numbers.iter().zip(digests.iter()) {
       // Charge for storage lookup
       handle.record_cost(GAS_STORAGE_LOOKUP)?;

       // Try attestation first
       if let Some(attestation) = Runtime::Attestations::get_attestation(
           query.chain_id,
           *digest,
       ) {
           if attestation.header_number != *block_num {
               return Ok(false);
           }
       } else {
           // Try checkpoint as fallback
           handle.record_cost(GAS_STORAGE_LOOKUP)?;

           if let Some(checkpoint_num) = Runtime::Checkpoints::get_checkpoint(
               query.chain_id,
               *digest,
           ) {
               if checkpoint_num != *block_num {
                   return Ok(false);
               }
           } else {
               // Neither attestation nor checkpoint found
               error!("Digest not found: {:?}", digest);
               return Ok(false);
           }
       }
   }
   ```

3. **Verify continuity to query height**
   - The continuity chain should connect to the queried block
   - Check that the chain ends at or near `query.height`
   ```rust
   if let Some(last_block) = continuity_chain.block_numbers.last() {
       // The last block in continuity + proof length should reach query height
       // You may need to calculate: last_block + continuity_proof_len >= query.height

       if *last_block > query.height {
           return Ok(false); // Chain extends beyond query
       }

       // Check gap is within acceptable range (depends on your continuity model)
       let gap = query.height - last_block;
       if gap > MAX_CONTINUITY_GAP {
           return Ok(false);
       }
   }
   ```

### Key Considerations

- **Runtime trait bounds**: You need access to `Attestations` and `Checkpoints` providers
  - These are already available through `Runtime: pallet_prover::Config` bound
  - Access them via: `<Runtime as pallet_prover::Config>::Attestations`
  - Or simpler: `Runtime::Attestations` (if trait is in scope)

- **Gas accounting**: Must charge for each storage lookup
  - Already provided: `handle.record_cost(GAS_STORAGE_LOOKUP)?;`

- **Attestation vs Checkpoint priority**:
  - Always try attestations first (they're more recent)
  - Fall back to checkpoints (they're finalized snapshots)

### Example Implementation Skeleton

```rust
#[cfg(not(feature = "runtime-benchmarks"))]
fn verify_continuity_chain(
    handle: &mut impl PrecompileHandle,
    continuity_chain: &ContinuityChain,
    query: &Query,
) -> Result<bool, PrecompileFailure>
where
    Runtime: pallet_prover::Config,
{
    // 1. Validate ordering
    for i in 0..continuity_chain.block_numbers.len().saturating_sub(1) {
        if continuity_chain.block_numbers[i] >= continuity_chain.block_numbers[i + 1] {
            return Ok(false);
        }
    }

    // 2. Verify each block
    for (block_num, digest) in continuity_chain.block_numbers.iter()
        .zip(continuity_chain.digests.iter())
    {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let mut found = false;

        // Try attestation
        if let Some(attestation) = <Runtime as pallet_prover::Config>::Attestations::get_attestation(
            query.chain_id,
            *digest,
        ) {
            if attestation.attestation.header_number == *block_num {
                found = true;
            }
        }

        // Try checkpoint if attestation not found
        if !found {
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            if let Some(checkpoint_num) = <Runtime as pallet_prover::Config>::Checkpoints::get_checkpoint(
                query.chain_id,
                *digest,
            ) {
                if checkpoint_num == *block_num {
                    found = true;
                }
            }
        }

        if !found {
            return Ok(false);
        }
    }

    // 3. Verify connection to query height
    if let Some(last_block) = continuity_chain.block_numbers.last() {
        // Add your continuity logic here
        // This depends on your specific continuity model
    }

    Ok(true)
}
```

---

## 3. Data Extraction (`extract_data_segments`)

**Location:** `src/lib.rs` around line 368

**Purpose:** Extract specific byte ranges from the verified transaction data according to the query layout.

### What You Need to Implement

```rust
fn extract_data_segments(
    tx_data: &[u8],
    query: &Query,
) -> Result<Vec<ResultSegment>, PrecompileFailure>
```

### Algorithm Steps

1. **Iterate through layout segments**
   - Each segment defines an offset and size
   ```rust
   let mut result_segments = Vec::new();

   for segment in &query.layout_segments {
       // Process each segment
   }
   ```

2. **Validate bounds**
   - Ensure offset + size doesn't exceed transaction data length
   ```rust
   let start = segment.offset as usize;
   let end = start + segment.size as usize;

   if end > tx_data.len() {
       error!("Segment out of bounds: offset={}, size={}, tx_len={}",
              segment.offset, segment.size, tx_data.len());
       let encoded_revert = encode_revert_message("Data segment out of bounds");
       return Err(PrecompileFailure::Revert {
           output: encoded_revert,
           exit_status: ExitRevert::Reverted,
       });
   }
   ```

3. **Extract bytes**
   - Get the byte slice from tx_data
   ```rust
   let bytes = &tx_data[start..end];
   ```

4. **Convert to H256 (32-byte chunks)**
   - Result must be H256 (32 bytes)
   - Pad with zeros if segment is smaller
   - Truncate if segment is larger (or handle specially)
   ```rust
   let mut padded = [0u8; 32];
   let copy_len = bytes.len().min(32);

   // Copy bytes to the right side (big-endian style)
   // Or left side, depending on your encoding
   padded[(32 - copy_len)..].copy_from_slice(&bytes[..copy_len]);
   // Or left-aligned: padded[..copy_len].copy_from_slice(&bytes[..copy_len]);

   result_segments.push(ResultSegment {
       offset: segment.offset,
       bytes: H256::from(padded),
   });
   ```

### Key Considerations

- **Alignment**: Decide how to align data in the 32-byte result
  - Right-aligned (big-endian): Common for numeric values
  - Left-aligned: Common for addresses and raw bytes

- **Size handling**:
  - If segment.size < 32: Pad with zeros
  - If segment.size == 32: Direct copy
  - If segment.size > 32: Decide whether to take first 32 bytes or error

- **RLP encoding**: If tx_data is RLP-encoded, you may need to decode first
  - For Ethereum transactions, you might need an RLP parser
  - Or expect pre-decoded data

### Example Implementation

```rust
fn extract_data_segments(
    tx_data: &[u8],
    query: &Query,
) -> Result<Vec<ResultSegment>, PrecompileFailure> {
    let mut result_segments = Vec::new();

    for segment in &query.layout_segments {
        // 1. Calculate bounds
        let start = segment.offset as usize;
        let end = start + segment.size as usize;

        // 2. Validate bounds
        if end > tx_data.len() {
            error!(
                "Layout segment out of bounds: offset={}, size={}, tx_data_len={}",
                segment.offset, segment.size, tx_data.len()
            );
            let encoded_revert = encode_revert_message("Data segment out of bounds");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        // 3. Extract bytes
        let bytes = &tx_data[start..end];

        // 4. Convert to H256
        let mut padded = [0u8; 32];

        if bytes.len() <= 32 {
            // Right-align (big-endian) - good for numbers
            let offset = 32 - bytes.len();
            padded[offset..].copy_from_slice(bytes);

            // Or left-align - good for addresses/raw bytes
            // padded[..bytes.len()].copy_from_slice(bytes);
        } else {
            // Handle oversized segments
            // Option 1: Take first 32 bytes
            padded.copy_from_slice(&bytes[..32]);

            // Option 2: Error out
            // let encoded_revert = encode_revert_message("Segment size exceeds 32 bytes");
            // return Err(PrecompileFailure::Revert { ... });
        }

        // 5. Add to results
        result_segments.push(ResultSegment {
            offset: segment.offset,
            bytes: H256::from(padded),
        });
    }

    Ok(result_segments)
}
```

---

## Testing Your Implementation

### Unit Tests

Run the test suite:
```bash
cargo test -p pallet-evm-precompile-native-query-verifier
```

The tests are in `src/tests.rs` and cover:
- Empty input validation
- Valid query success case
- Gas cost calculations
- Out of bounds handling
- Multiple segments

### Adding More Tests

Add tests for your specific implementation:

```rust
#[test]
fn test_merkle_proof_verification() {
    ExtBuilder::default().build().execute_with(|| {
        // Create a known Merkle tree
        // Verify proof works correctly
    });
}

#[test]
fn test_continuity_chain_with_attestations() {
    ExtBuilder::default().build().execute_with(|| {
        // Set up attestations in storage
        // Verify continuity chain
    });
}
```

### Integration Testing

1. Deploy to a test network
2. Call from Solidity using the example contracts
3. Verify gas costs match expectations
4. Test with real Ethereum transaction data

---

## Runtime Integration

Once implementation is complete, integrate into the runtime:

### 1. Add to Runtime Precompiles

Edit `runtime/src/precompiles.rs`:

```rust
use pallet_evm_precompile_native_query_verifier::NativeQueryVerifierPrecompile;

impl PrecompileSet for GluwaPrecompiles<R> {
    fn execute(&self, handle: &mut impl PrecompileHandle) -> Option<PrecompileResult> {
        match handle.code_address() {
            // ... existing precompiles ...
            a if a == hash(3050) => Some(NativeQueryVerifierPrecompile::<Runtime>::execute(handle)),
            _ => None,
        }
    }
}
```

### 2. Update Workspace Cargo.toml

```toml
[workspace.dependencies]
pallet-evm-precompile-native-query-verifier = { path = "precompiles/native-query-verifier", default-features = false }
```

### 3. Update Runtime Cargo.toml

```toml
[dependencies]
pallet-evm-precompile-native-query-verifier = { workspace = true }

[features]
std = [
    # ... other features ...
    "pallet-evm-precompile-native-query-verifier/std",
]
```

---

## Next Steps

1. **Implement Merkle verification** - Start here, it's the most critical
2. **Implement data extraction** - Easier, good to get quick wins
3. **Implement continuity chain** - More complex, may need iteration
4. **Test thoroughly** - Add comprehensive tests for each function
5. **Benchmark gas costs** - Ensure they match actual computational cost
6. **Integration test** - Deploy and test with real data

---

## Resources

### Creditcoin3 Internal
- Existing proof verifier: `precompiles/proof-verifier/src/lib.rs`
- Attestation provider: `primitives/attestor/src/provider.rs`
- Query primitives: `primitives/pallet-prover/src/lib.rs`

### External References
- [Merkle Proofs](https://en.wikipedia.org/wiki/Merkle_tree)
- [Ethereum Yellow Paper](https://ethereum.github.io/yellowpaper/paper.pdf) - Section 4.3 (Transaction Tree)
- [Substrate Documentation](https://docs.substrate.io/)
- [EVM Precompiles](https://www.evm.codes/precompiled)

---

## Getting Help

If you get stuck:
1. Check the existing `proof-verifier` precompile for patterns
2. Look at how attestations are accessed in other pallets
3. Review the test suite for expected behavior
4. Check Substrate/Frontier documentation for precompile best practices

Good luck with the implementation! 🚀
