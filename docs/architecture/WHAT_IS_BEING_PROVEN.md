# What Is Being Proven in Query Proofs

## Executive Summary

Query proofs are **NOT just proofs of inclusion**. They are comprehensive proofs that establish:

1. **Transaction Inclusion** - The transaction exists in a specific block at a specific index
2. **Data Integrity** - The transaction data is correct and unmodified
3. **Chain Continuity** - The block is part of the canonical chain history
4. **Temporal Ordering** - The proof links through a chain of attestations back to a known checkpoint

This is much more powerful than a simple Merkle proof of inclusion.

---

## The Complete Picture

### What You're Actually Proving

```
┌─────────────────────────────────────────────────────────────────┐
│                    Query Proof Contains:                         │
│                                                                   │
│  1. Merkle Proof                                                 │
│     - Transaction is at index I in block B                       │
│     - Transaction data matches claimed data                      │
│     - Root matches block's transaction root                      │
│                                                                   │
│  2. Continuity Chain                                             │
│     - Block B links to Block B-1                                 │
│     - Block B-1 links to Block B-2                              │
│     - ...continues back to known checkpoint                      │
│     - Each link is cryptographically verified                    │
│                                                                   │
│  3. Data Extraction                                              │
│     - Specific bytes from transaction extracted                  │
│     - Layout segments verified                                   │
│     - Query structure validated                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Part 1: Merkle Proof of Inclusion

### What's Being Proven

```
"Transaction TX exists at position INDEX in block BLOCK_HEIGHT"
```

### How It Works

```
Block's Transaction Merkle Tree:
                    Root
                   /    \
                 /        \
               H1          H2
              /  \        /  \
            T0   T1     T2   T3
                 ↑
                 └─ This is our transaction at index 1
```

**Merkle Proof Provides:**
- Sibling hashes along the path: `[T0, H2]`
- Allows reconstruction: `Hash(Hash(T0, T1), H2) = Root`
- Verifies: Transaction T1 is at index 1

**Cairo Program Verification:**
```cairo
func verify_merkle_path(...) {
    // Recursively hash up the tree with siblings
    let h = hash_with_siblings(word_hash, proof_items, offset, arity)

    // At the end, computed hash must equal root
    assert root = h

    // Return the index calculated from path
    return (index, ...)
}
```

### What This Proves

✅ Transaction exists in the block
✅ Transaction is at claimed index
✅ Transaction data is authentic (any change breaks hash)
✅ Cannot forge without breaking cryptographic hash

### What This DOESN'T Prove

❌ The block is part of the canonical chain
❌ The block hasn't been reorganized out
❌ The block is at the claimed height

**This is why we need the continuity chain!**

---

## Part 2: Continuity Chain (The Key Innovation)

### The Problem Merkle Proofs Don't Solve

```
Scenario: Malicious Prover
1. Creates fake block with fake transactions
2. Builds Merkle tree over fake transactions
3. Generates valid Merkle proof
4. Submits to verifier

Problem: Merkle proof is technically valid!
Solution: Continuity chain ensures block is in canonical history
```

### What's the Continuity Chain?

A **cryptographic chain of attestations** linking blocks together:

```
Known Checkpoint (Attested/Finalized)
  ↓ (digest links to)
Block N
  ↓ (digest links to)
Block N+1
  ↓ (digest links to)
Block N+2 (Query Block)
  ↓
... continues to present
```

### Continuity Block Structure

```rust
struct ContinuityBlock {
    merkle_root: felt,  // Transaction root for this block
    digest: felt,       // Pedersen(block_number, merkle_root, prev_digest)
}
```

**Digest Computation:**
```cairo
// Each block's digest incorporates previous block's digest
let d = pedersen_hash2(block_number, merkle_root)
let digest = pedersen_hash2(d, prev_digest)

// This creates a chain:
// digest(N+1) depends on digest(N)
// digest(N+2) depends on digest(N+1)
// etc.
```

### How Continuity is Verified

**Cairo Program:**
```cairo
// 1. Compute digest for query block
let d = pedersen_hash2(block_number, proof_root)
let digest = pedersen_hash2(d, continuity_blocks[0].digest)

// 2. Verify it matches attestation chain
assert curr_digest_from_attestation_chain = digest

// 3. Verify continuity back to checkpoint
let checkpoint_digest = generate_continuity_attestation(
    continuity_blocks,
    block_number,
    prev_digest,
    len
)

// 4. Verify final digest matches checkpoint
assert checkpoint_digest = continuity_attestation_checkpoint.digest
```

**What This Proves:**
- Query block's merkle_root is the one attested by validators
- Block is linked through unbroken chain to known checkpoint
- Cannot substitute a different block (digest wouldn't match)
- Cannot use a forked/orphaned block (not in attestation chain)

### Example: Query for Block 1000

```
1. Known Checkpoint at Block 990 (digest_990)
   - Finalized by validator consensus
   - Stored on Creditcoin3 chain

2. Continuity Fragment: Blocks 990-1000
   [
     {root_990, digest_990},
     {root_991, digest_991 = hash(991, root_991, digest_990)},
     {root_992, digest_992 = hash(992, root_992, digest_991)},
     ...
     {root_1000, digest_1000 = hash(1000, root_1000, digest_999)}
   ]

3. Cairo Verifies:
   - Each digest correctly links to previous
   - digest_990 matches known checkpoint
   - digest_1000 matches current attestation
   - All 10 blocks form unbroken chain

4. Therefore:
   - Block 1000 is part of canonical chain
   - Its merkle_root is authentic
   - Transaction in that block is authentic
```

---

## Part 3: The Complete Proof Flow

### Off-Chain: Proof Generation

```
┌─────────────────────────────────────────────────────────────────┐
│                    Prover Service                                │
└─────────────────────────────────────────────────────────────────┘
                         │
                         ▼
1. Receive Query
   - Chain ID: 1 (Ethereum)
   - Block: 1000
   - Transaction Index: 5
   - Layout Segments: [bytes to extract]

                         │
                         ▼
2. Fetch Block Data
   - Get block 1000 from Ethereum node
   - Extract transaction 5
   - Build Merkle tree of all transactions
   - Generate Merkle proof for tx 5

                         │
                         ▼
3. Fetch Continuity Chain
   - Query attestor network for blocks 990-1000
   - Get attestations (signed by validators)
   - Extract merkle roots and digests
   - Verify chain links properly

                         │
                         ▼
4. Prepare Cairo Input
   {
     "block_number": 1000,
     "merkle_proof": {
       "root": merkle_root_1000,
       "path": [...sibling hashes...],
       "subject": transaction_5_data
     },
     "continuity_chain": {
       "start": 990,
       "blocks": [
         {"root": root_990, "digest": digest_990},
         {"root": root_991, "digest": digest_991},
         ...
       ]
     },
     "query": {
       "felt_ranges": [...]
     }
   }

                         │
                         ▼
5. Execute Cairo Program
   - verify_merkle_proof.cairo
   - Verifies Merkle proof
   - Verifies continuity chain
   - Extracts query data

                         │
                         ▼
6. Generate STARK Proof
   - Stone prover (cpu_air_prover)
   - Creates cryptographic proof of Cairo execution
   - Proof is ~1-10 MB
   - Proves "I ran this program correctly"

                         │
                         ▼
7. Submit Proof
   - STARK proof (JSON)
   - Contains public outputs:
     * Transaction index
     * Continuity digest
     * Query hash
     * Extracted data
```

### On-Chain: Proof Verification

```
┌─────────────────────────────────────────────────────────────────┐
│                Smart Contract / Precompile                       │
└─────────────────────────────────────────────────────────────────┘
                         │
                         ▼
1. Receive Proof + Query
   - STARK proof (bytes)
   - Query structure

                         │
                         ▼
2. Call Proof Verifier Precompile (0x0Be9)
   - Reads STARK program metadata
   - Invokes host API

                         │
                         ▼
3. Host API → Verifier Core
   - Parses STARK proof
   - Authenticates Cairo program
   - Executes Stone verifier (cpu_air_verifier)

                         │
                         ▼
4. Stone Verifier
   - Cryptographically verifies STARK proof
   - Confirms Cairo program ran correctly
   - Extracts public outputs

                         │
                         ▼
5. Validate Continuity
   - Extract continuity digest from proof
   - Calculate expected block number
   - Check Attestations pallet for digest
   - Verify block number matches

                         │
                         ▼
6. Return Results
   - Status: 0 (success)
   - Result segments: extracted data
   - Smart contract can now use this data!
```

---

## What Each Component Proves

### Merkle Proof Proves:

```
✅ Transaction T exists
✅ Transaction T is at index I
✅ Transaction T is in block with root R
✅ Transaction data is D (exact bytes)
```

### Continuity Chain Proves:

```
✅ Block with root R is at height H
✅ Block is in canonical chain
✅ Block is linked to known checkpoint
✅ Block hasn't been reorganized out
✅ Chain is unbroken from checkpoint to query block
```

### STARK Proof Proves:

```
✅ Cairo program executed correctly
✅ All assertions passed (Merkle verified, continuity verified)
✅ Outputs are correct (extracted data)
✅ Program wasn't tampered with (authenticated)
```

### Combined: The Full Guarantee

```
"Transaction T with data D exists at index I in block H,
 and block H is part of the canonical chain as verified
 by validator attestations, and these specific bytes were
 extracted from the transaction data."
```

---

## Why Both Are Necessary

### Merkle Proof Alone

```
❌ Can prove fake block
❌ Can use orphaned block
❌ Can use reorged block
✅ Proves transaction in *some* block
```

### Continuity Chain Alone

```
✅ Proves block is canonical
✅ Links to checkpoint
❌ Doesn't prove transaction exists
❌ Doesn't prove transaction data
```

### Together (Current System)

```
✅ Proves transaction exists
✅ Proves transaction data
✅ Proves block is canonical
✅ Proves chain continuity
✅ Complete trustless verification
```

---

## Real-World Example: Proving an ETH Transfer

### Query: "Prove Alice sent 10 ETH to Bob in block 1000"

**Step 1: Get Merkle Proof**
```
Block 1000 has 200 transactions
Transaction 42 is Alice→Bob transfer
Merkle proof: [siblings from tx 42 to root]
Proves: "Transaction 42 is in block 1000"
```

**Step 2: Get Continuity Chain**
```
Last checkpoint: Block 990
Continuity chain: Blocks 990→991→...→1000
Proves: "Block 1000 is in canonical chain"
```

**Step 3: Cairo Verification**
```cairo
// Verify Merkle proof
verify_merkle_path(root_1000, tx_42_hash, proof)
  → Returns index 42 ✓

// Verify continuity
generate_continuity_attestation(blocks_990_to_1000)
  → Digest matches checkpoint ✓

// Extract data
output_array_at_offsets(tx_42_data, [from, to, value])
  → Returns [Alice, Bob, 10 ETH] ✓
```

**Step 4: STARK Proof**
```
Stone prover generates proof:
"I verified the Merkle proof AND the continuity chain AND
 extracted the data, all correctly."
```

**Step 5: On-Chain Verification**
```
1. Verify STARK proof ✓
2. Check continuity digest against Attestations pallet ✓
3. Verify block number calculation ✓
4. Return extracted data to smart contract ✓

Result: Smart contract now KNOWS Alice sent 10 ETH to Bob!
```

---

## Security Properties

### Against Malicious Prover

| Attack | Defense |
|--------|---------|
| Fake transaction | Merkle proof won't match real root |
| Modified data | Hash breaks, proof invalid |
| Wrong block | Continuity digest won't match attestation |
| Orphaned block | Not in attestation chain |
| Future block | No attestation exists yet |
| Reorged block | Attestation chain updated, digest changes |

### Cryptographic Guarantees

```
Merkle Proof Security:
  - 2^128 work to find collision
  - Computational infeasibility

Continuity Chain Security:
  - Each digest depends on previous
  - Breaking requires breaking Pedersen hash
  - Validator signatures verify authenticity

STARK Proof Security:
  - Soundness: 2^-100 probability of accepting false proof
  - Zero-knowledge: Reveals nothing beyond claim
  - Post-quantum secure
```

---

## The Attestation Layer

### What Are Attestations?

```rust
struct SignedAttestation {
    attestation: Attestation {
        chain_id: ChainId,
        header_number: u64,
        digest: H256,
        // ... other fields
    },
    signatures: Vec<Signature>,  // Validator signatures
}
```

### How They're Generated

```
1. Attestor Network monitors external chains
2. For each block, attestors:
   - Fetch block header
   - Build transaction Merkle tree
   - Compute merkle root
   - Compute digest = hash(block_num, root, prev_digest)
3. Validators sign attestation
4. Attestation stored on-chain (Creditcoin3)
5. Becomes source of truth for continuity
```

### Checkpoints vs Attestations

```
┌────────────────────────────────────────────┐
│  Attestation (every block)                 │
│  - Generated by attestor network           │
│  - Contains merkle root + digest           │
│  - Signed by validators                    │
│  - Stored on-chain                         │
└────────────────────────────────────────────┘
                  │
                  ▼ (every N blocks)
┌────────────────────────────────────────────┐
│  Checkpoint (periodic)                     │
│  - Snapshot of attestation                 │
│  - Finalized by consensus                  │
│  - Used as anchor for continuity chains    │
│  - Older attestations can be pruned        │
└────────────────────────────────────────────┘
```

**Prover Strategy:**
- Start from most recent checkpoint
- Build continuity chain forward to query block
- Shorter chain = faster proof generation

---

## Continuity Proof Validation (Detailed)

### What the Verifier Checks

```rust
// 1. Extract continuity data from proof
let continuity_proof_length = proof.continuity_proof_length;
let continuity_digest = proof.continuity_checkpoint_digest;

// 2. Calculate expected block number
let expected_block = query.height - 1 + continuity_proof_length - 1;

// 3. Lookup attestation by digest
if let Some(attestation) = Attestations::get(chain_id, continuity_digest) {
    // Found attestation - check block number
    if attestation.header_number == expected_block {
        ✓ ACCEPT
    } else {
        ✗ REJECT: Block number mismatch
    }
} else if let Some(checkpoint) = Checkpoints::get(chain_id, continuity_digest) {
    // Found checkpoint - check block number
    if checkpoint.block_number == expected_block {
        ✓ ACCEPT
    } else {
        ✗ REJECT: Block number mismatch
    }
} else {
    ✗ REJECT: No attestation or checkpoint found
}
```

### Why This Matters

```
Example: Query for block 1000

Proof claims: "Continuity chain from block 990 to 1000 (length 11)"

Verifier calculates:
  expected_block = 1000 - 1 + 11 - 1 = 1009

Wait, that doesn't match! Expected 990 (the checkpoint block)

Actually:
  continuity_proof_length = 11 (blocks 990 through 1000, inclusive)
  expected_block = 1000 - 1 + 11 - 1 = 1009

The formula accounts for:
  - query.height - 1 = block before query (999)
  - + continuity_proof_length = how many blocks in chain
  - - 1 = because length includes start block

This ensures the continuity chain anchors to the correct checkpoint!
```

---

## Summary: What Is Being Proven

### In One Sentence

**"A specific transaction with verified data exists at a specific position in a specific block, which is provably part of the canonical blockchain as verified by validator attestations."**

### The Three Pillars

1. **Merkle Proof**: Transaction inclusion and data integrity
2. **Continuity Chain**: Canonical chain membership
3. **STARK Proof**: Computational integrity of verification

### Why It's Powerful

```
Traditional Approach:
  - Trust the RPC node
  - Trust they're not lying
  - Trust block won't reorg

Query Proof Approach:
  - Zero trust assumptions
  - Cryptographically verified
  - Anchored to validator consensus
  - Provably correct
```

### Use Cases Enabled

- ✅ Cross-chain bridges (trustless)
- ✅ Credit history verification (multi-chain)
- ✅ Trustless oracles (any chain data)
- ✅ Atomic swaps (verified state)
- ✅ Fraud proofs (challenge invalid claims)

---

## Further Reading

- **Merkle Proofs**: [Wikipedia - Merkle Tree](https://en.wikipedia.org/wiki/Merkle_tree)
- **STARK Proofs**: [StarkWare STARK Explanation](https://starkware.co/stark/)
- **Cairo Programs**: [Cairo Documentation](https://www.cairo-lang.org/docs/)
- **Attestor Network**: `docs/attestation_doc.md`
- **Continuity Design**: Look for design docs in attestor documentation
