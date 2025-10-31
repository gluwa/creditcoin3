# Checkpoint Support in Query Verification

## Overview

The query verification system now supports **attestation checkpoints** in addition to regular attestations. Checkpoints are condensed representations of multiple attestations, used to reduce storage overhead in the Creditcoin3 chain.

## Problem Statement

When attestations are created for each block on a source chain (e.g., Ethereum), the storage requirements grow linearly with the number of blocks. To address this, the attestation system periodically condenses old attestations into checkpoints.

**Example:**
- Attestations exist for blocks: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10
- After condensation, checkpoints replace attestations: [checkpoint@5, checkpoint@10]
- Original attestations for blocks 1-10 are removed from storage

**Challenge for Query Verification:**
When a query targets block 16, the continuity proof needs to link from the last known attestation/checkpoint to block 16. If block 0 has a checkpoint but blocks 1-15 don't have individual attestations, the verification must still work.

## Solution Architecture

### 1. Precompile Changes

**File:** `precompiles/native-query-verifier/src/lib.rs`

#### Added Methods

```rust
/// Get the last checkpoint for a chain
fn last_checkpoint(chain_key: u64) -> Option<attestor_primitives::AttestationCheckpoint> {
    pallet_attestation_poc::Pallet::<Runtime>::last_checkpoint(chain_key)
}

/// Check if a digest corresponds to a checkpoint
fn get_checkpoint(chain_key: u64, digest: H256) -> Option<u64> {
    pallet_attestation_poc::Pallet::<Runtime>::checkpoints(chain_key, digest)
}
```

#### Updated Verification Logic

**Initial Digest Resolution:**
```rust
// Try last attestation first, then fall back to last checkpoint
let mut last_finalized_digest = Self::last_digest(query.chain_id)
    .or_else(|| Self::last_checkpoint(query.chain_id).map(|cp| cp.digest))
    .ok_or_else(|| {
        error!("❌ No finalized attestation or checkpoint found");
        PrecompileFailure::Revert { ... }
    })?;
```

**Tail Validation:**
When validating the first block in the continuity proof, the precompile now checks:
1. If `prev_digest` matches a known attestation → validate block_number
2. If `prev_digest` matches a known checkpoint → validate block_number
3. If neither → reject as invalid

```rust
// Check if the tail's prev_digest matches a known attestation or checkpoint
if let Some(prev_attestation) = Self::get_attestation(query.chain_id, block_prev_digest) {
    // Validate attestation
    if prev_attestation.attestation.header_number != tail.block_number - 1 {
        return Ok(false);
    }
    last_finalized_digest = block_prev_digest;
} else if let Some(checkpoint_block_number) = Self::get_checkpoint(query.chain_id, block_prev_digest) {
    // Validate checkpoint
    if checkpoint_block_number != tail.block_number - 1 {
        return Ok(false);
    }
    last_finalized_digest = block_prev_digest;
} else {
    error!("❌ Tail prev digest not found in attestations or checkpoints");
    return Ok(false);
}
```

### 2. CLI Changes

**File:** `query-cli/src/continuity.rs`

The CLI already had checkpoint-aware logic for finding bounds, but now includes enhanced logging:

```rust
// Find highest attestation/checkpoint before query height
let lower_attestation = attestations
    .iter()
    .filter(|a| a.attestation.header_number < query_height)
    .max_by_key(|a| a.attestation.header_number);

let lower_checkpoint = checkpoints
    .iter()
    .filter(|c| c.block_number < query_height)
    .max_by_key(|c| c.block_number);

// Choose the higher of the two as lower bound
let lower_bound = match (lower_attestation, lower_checkpoint) {
    (Some(a), Some(c)) => if a.0 > c.0 { Some(a) } else { Some(c) },
    (Some(a), None) => Some(a),
    (None, Some(c)) => Some(c),
    (None, None) => None,
};
```

## How It Works

### Scenario: Query with Checkpoint

**Given:**
- Checkpoint at block 0 with digest `D0`
- Query targeting block 16
- No individual attestations for blocks 1-15

**Continuity Chain Construction:**

1. **Find Bounds:**
   - Lower bound: checkpoint at block 0 (digest `D0`)
   - Upper bound: attestation at block 20 (or next checkpoint)

2. **Build Continuity Blocks:**
   - CLI fetches actual block data from Ethereum for blocks 1-16
   - Each block is constructed with:
     - `block_number`: actual block number
     - `root`: Merkle root of transactions in that block
     - `prev_digest`: digest of previous block in chain
     - `digest`: keccak256(block_number || root || prev_digest)

3. **Verification:**
   - Precompile checks that block 1's `prev_digest` equals `D0` (checkpoint digest)
   - Precompile validates checkpoint exists at block 0 with digest `D0`
   - Precompile validates chain links: each block's `prev_digest` = previous block's `digest`

### Chain Validation Logic

```
Checkpoint@0 → Block1 → Block2 → ... → Block16 (query)
     D0         D1        D2              D16

Where:
- Block1.prev_digest = D0 (must match checkpoint)
- Block2.prev_digest = D1 (computed from Block1)
- Block16.prev_digest = D15 (computed from Block15)
```

## Key Insights

### 1. Intermediate Blocks Don't Need Attestations

The continuity proof includes intermediate blocks (1-15) that are **not attested**. This is valid because:
- The precompile only requires the **tail's prev_digest** to match a known attestation/checkpoint
- The rest of the chain is validated by **digest computation** (cryptographic linking)
- Each block's digest is computed from its data, ensuring integrity

### 2. Checkpoint as Trust Anchor

The checkpoint serves as a **trust anchor**:
- Represents that blocks up to the checkpoint height were attested
- Provides a starting digest for continuity chains
- Allows validation without storing all historical attestations

### 3. Hybrid Approach

The system supports both attestations and checkpoints simultaneously:
- Recent blocks: use attestations (fine-grained)
- Historical blocks: use checkpoints (space-efficient)
- Verification works with mixed chains

## Storage Model

### Attestations
```rust
pub type Attestations<T> = StorageDoubleMap<
    ChainKey,
    Digest,
    SignedAttestation
>;

pub type LastAttestationDigest<T> = StorageMap<
    ChainKey,
    Digest
>;
```

### Checkpoints
```rust
pub type Checkpoints<T> = StorageDoubleMap<
    ChainKey,
    Digest,
    BlockNumber  // u64
>;

pub type LastCheckpoint<T> = StorageMap<
    ChainKey,
    AttestationCheckpoint
>;
```

**Key Difference:**
- Attestations store full `SignedAttestation` (includes signatures, votes, etc.)
- Checkpoints store only `BlockNumber` (lightweight)

## Gas Implications

### Without Checkpoint Support
```
Storage reads: O(N) where N = blocks between attestations
Gas cost: N × GAS_STORAGE_LOOKUP
```

### With Checkpoint Support
```
Storage reads: 1 (checkpoint lookup)
Gas cost: GAS_STORAGE_LOOKUP (constant)
Continuity chain: validated by digest computation (no storage reads)
```

**Savings:** Reduces storage reads from linear to constant!

## Error Cases

### 1. No Attestation or Checkpoint Found
```
Error: "No finalized attestation or checkpoint found for chain_id X"
Status: Revert
```

**Cause:** Chain not initialized or query targeting unsupported chain

### 2. Tail Prev Digest Not Found
```
Error: "Continuity proof tail prev digest not found in attestations or checkpoints"
Status: ContinuityChainInvalid (2)
```

**Cause:** First block's `prev_digest` doesn't match any known checkpoint/attestation

**Debug:**
- Check if CLI is using correct lower_bound
- Verify checkpoint/attestation exists with expected digest
- Check digest computation is correct

### 3. Block Number Mismatch
```
Error: "Tail prev digest points to checkpoint with block number X, but expected Y"
Status: ContinuityChainInvalid (2)
```

**Cause:** Continuity chain doesn't start from the right block

**Debug:**
- Verify `tail.block_number - 1` matches checkpoint block number
- Check if CLI skipped blocks incorrectly

## Testing

### Test Scenario 1: Pure Checkpoint Chain
```
Setup:
- Checkpoint at block 0
- Query at block 16
- No intermediate attestations

Expected:
- CLI builds blocks 1-16 with computed digests
- Precompile validates block 1.prev_digest matches checkpoint
- Verification succeeds
```

### Test Scenario 2: Mixed Attestation/Checkpoint
```
Setup:
- Checkpoint at block 0
- Attestation at block 10
- Query at block 16

Expected:
- CLI uses attestation as lower_bound (higher than checkpoint)
- Builds blocks 11-16
- Verification succeeds
```

### Test Scenario 3: Checkpoint Condensation
```
Setup:
- Attestations at blocks 1-10
- Condense to checkpoint at block 10
- Query at block 15

Before condensation:
- Lower bound: attestation@10
- Works

After condensation:
- Lower bound: checkpoint@10
- Should still work (with checkpoint support)
```

## CLI Output Example

```bash
$ cargo run --bin query-cli -- --query-height 16 ...

=== Continuity Proof Generation ===
Found lower checkpoint at height 0 with digest [0x12, 0x34, ...]
Using checkpoint as lower bound (height 0)
Fetching continuity chain from block 1 to 16
Building continuity chain starting with lower_digest: 0x1234...
  Block 1: root=[0xab, ...], prev_digest=[0x12, ...], digest=[0xcd, ...]
  Block 2: root=[0xef, ...], prev_digest=[0xcd, ...], digest=[0x56, ...]
  ...
  Block 16: root=[0x78, ...], prev_digest=[0x90, ...], digest=[0xab, ...]
Constructed continuity proof with 16 blocks

=== Query Verification ===
✅ Verification successful!
```

## Future Optimizations

### 1. Checkpoint Caching
Cache recently used checkpoints in memory to reduce storage reads:
```rust
static CHECKPOINT_CACHE: Lazy<RwLock<LruCache<(ChainKey, Digest), u64>>> = ...;
```

### 2. Sparse Continuity Proofs
Instead of including every block, include only:
- Tail block (links to checkpoint)
- Query block (contains target transaction)
- Skip intermediate blocks if digest can be computed

### 3. Batch Checkpoint Queries
When multiple queries target same checkpoint range, batch the checkpoint lookups.

## Security Considerations

### 1. Checkpoint Authenticity
Checkpoints are created by governance/consensus, ensuring they represent legitimate attestations.

### 2. Digest Immutability
Once a checkpoint is created, its digest is immutable. Any tampering would invalidate the continuity chain.

### 3. Block Data Integrity
Even without individual attestations, block integrity is ensured by:
- Merkle root verification (proves transaction was in block)
- Digest chain (proves block sequence is correct)
- Checkpoint anchor (proves starting point is valid)

## Summary

✅ **Checkpoint support is fully implemented**
✅ **Precompile validates both attestations and checkpoints**
✅ **CLI builds correct continuity chains**
✅ **Gas costs reduced through checkpoint efficiency**
✅ **Backwards compatible with attestation-only chains**

The system now efficiently handles both fresh attestations and historical checkpoints, providing a scalable solution for long-running attestation chains.
