# Why Cairo Works with Felts Instead of Bytes

## TL;DR

Cairo doesn't work with felts by choice—**it's a fundamental requirement of STARK proofs**. Transaction data is stored as felts in the Merkle tree, and Cairo can only read felts. When users query byte ranges, we must convert those ranges to felt indices, have Cairo read the felts, then extract the requested bytes in Rust.

## Background: What is a Felt?

A **felt** (field element) is a number in a finite field:
- Size: 252 bits (~31.5 bytes)
- Practical use: 248 bits (31 bytes) to stay within safe bounds
- Used by: All STARK-based systems (Cairo, StarkNet, etc.)

## Why STARKs Require Felts

STARK proofs work by encoding computation as **polynomial arithmetic over finite fields**. This is the mathematical foundation of STARKs:

```
Every operation must be a field element operation:
  ✅ read felt[6]              (valid field operation)
  ❌ read byte[192]            (no such thing in field arithmetic)
```

This isn't a Cairo design choice—it's what makes STARKs mathematically possible.

## The Data Storage Reality

### Transactions are Stored as Felts

When transaction data enters the Merkle tree, raw bytes are **pre-converted to felts**:

```python
# From verify_merkle_proof.cairo
def bytes_to_felt_array(bytes):
    FELT_SIZE = 31
    return [int.from_bytes(bytes[i : i + FELT_SIZE], "big")
            for i in range(0, len(bytes), FELT_SIZE)]
```

**Example:**
```
Raw transaction: 100 bytes
    ↓
Converted to: [felt₀, felt₁, felt₂, felt₃]
    ↓
Stored in Merkle tree as 4 field elements
```

### Cairo Can Only Read Felts

```
┌──────────────────────────────────────┐
│      Merkle Tree (STARK Format)      │
│                                      │
│  Leaf: [felt₀, felt₁, felt₂, ...]   │
│                                      │
│  ⚠️  NOT stored as bytes!            │
│  ⚠️  Cairo cannot read bytes!        │
│  ✅  Cairo reads felts directly      │
└──────────────────────────────────────┘
```

## The Query Translation Problem

### What Users Request

```rust
// "Give me bytes 192-223 from this transaction"
LayoutSegment { offset: 192, size: 32 }
```

### What Cairo Actually Has

The transaction is stored as felts, where each felt holds 31 bytes:

```
Felt[0] = bytes [0..31]     (31 bytes)
Felt[1] = bytes [31..62]     (31 bytes)
...
Felt[6] = bytes [186..217]   (31 bytes)  ← Contains bytes 192-216
Felt[7] = bytes [217..248]   (31 bytes)  ← Contains bytes 217-223
...
```

### The Necessary Conversion

```rust
// Step 1: Convert byte range to felt indices
byte_offset / 31 = felt_start_index
(byte_offset + size - 1) / 31 = felt_end_index

// For bytes [192..224):
192 / 31 = 6        (felt start)
223 / 31 = 7        (felt end)

// Step 2: Cairo reads felts [6, 7]
cairo_output = [felt₆, felt₇]  // 62 bytes total

// Step 3: Rust extracts the exact byte range
// felt₆ starts at byte 186 (6 × 31)
// We want bytes 192-223
// Skip first 6 bytes of felt₆ (192 - 186 = 6)
// Take 32 bytes spanning felts 6 and 7
result = extract_bytes(cairo_output, skip: 6, take: 32)
```

## The Complete Proving Flow

### 1. Prover Preparation

```
User queries: "bytes 192-223"
    ↓
Convert to felt indices: [6, 7]
    ↓
Hash felt indices (for verification)
    ↓
Send to Cairo prover
```

### 2. Cairo Execution

```cairo
// Cairo can only do this:
func read_transaction_data(
    felt_array: felt*,
    felt_offset: felt,
    felt_count: felt
) -> felt* {
    // Direct array access - single STARK constraint per felt
    return felt_array + felt_offset
}

// Cairo CANNOT do this efficiently:
func read_bytes(byte_offset: felt) -> felt {
    // Would require expensive bit operations in field arithmetic
    // Each byte read = 100+ STARK constraints
    // ❌ Proof would be massive
}
```

### 3. Verifier Extraction

```rust
// Rust receives STARK proof containing felts
let cairo_output: Vec<Felt252> = proof.public_outputs;

// Convert felts back to bytes (zero cost, outside proof)
let felt_bytes: Vec<u8> = cairo_output
    .iter()
    .flat_map(|f| f.to_bytes_be()[..31])
    .collect();

// Extract the exact byte range requested
let result = extract_segment(&felt_bytes, original_byte_query);
```

## Why Not Extract Bytes in Cairo?

### Option A: Extract in Cairo ❌

```cairo
// Cairo would need to:
// 1. Read felt containing the byte
// 2. Convert felt to binary representation
// 3. Extract specific bits using field arithmetic
// 4. Mask and shift to get the byte
//
// Cost per byte: ~100+ STARK constraints
// For 32 bytes: ~3,200 constraints
// Proof size: MASSIVE
```

### Option B: Extract in Rust ✅

```rust
// 1. Cairo reads 2 felts (2 constraints)
// 2. Rust extracts bytes (FREE - outside proof)
//
// Total cost: 2 constraints
// Proof size: Minimal
//
// Savings: ~1,600x smaller proof
```

## Real-World Example: ERC20 Transfer

### User Query

```solidity
// Query three 32-byte fields:
// - from address (bytes 192-223)
// - to address (bytes 224-255)
// - amount (bytes 448-479)
```

### Felt Conversion

```rust
// Convert each byte range to felt indices:
[192..224) → felts [6, 7]      (6×31=186, 7×31=217)
[224..256) → felts [7, 8]      (7×31=217, 8×31=248)
[448..480) → felts [14, 15]    (14×31=434, 15×31=465)

// Merge overlapping ranges:
Felts needed: [6, 7, 8, 14, 15]  (5 felts = 155 bytes)
```

### Cairo Proof

```cairo
// Cairo reads 5 felts (5 STARK constraints)
output[0] = felt_array[6]
output[1] = felt_array[7]
output[2] = felt_array[8]
output[3] = felt_array[14]
output[4] = felt_array[15]

// Proof contains 155 bytes of felt data
```

### Rust Extraction

```rust
// Extract from address (bytes 192-223)
// Comes from felts [6, 7] = bytes [186..248)
// Skip 6 bytes (192-186), take 32 bytes
let from = extract(&felts[0..2], 6, 32);

// Extract to address (bytes 224-255)
// Comes from felts [7, 8] = bytes [217..279)
// Skip 7 bytes (224-217), take 32 bytes
let to = extract(&felts[1..3], 7, 32);

// Extract amount (bytes 448-479)
// Comes from felts [14, 15] = bytes [434..496)
// Skip 14 bytes (448-434), take 32 bytes
let amount = extract(&felts[3..5], 14, 32);
```

**Result:** 96 bytes of user data proven with only 5 felt reads instead of 96 individual byte operations.

## Why Hash the Felt Indices?

The felt index hash is a **separate verification step** from the felt conversion itself:

```rust
// 1. Conversion (MANDATORY - required by STARK constraints)
byte_segments → felt_segments

// 2. Hashing (OPTIONAL - verification only)
query_hash = hash(felt_segments)
```

### Purpose of the Hash

The hash ensures the prover and verifier computed the same felt indices:

```
✅ With hash:
Prover: "I'll read felts [6, 7]"
Hash: 0xabc123...
Cairo: Reads felts [6, 7]
Verifier: Computes felts [6, 7], hash matches ✓

❌ Without hash (potential bug):
Prover: "I'll read felts [5, 6]" (bug in conversion!)
Cairo: Reads felts [5, 6]
Verifier: Expects felts [6, 7]
No immediate detection of mismatch!
```

The hash provides **early detection** of conversion bugs or malicious provers, though the verification layer would catch discrepancies regardless.

## What's Optional, What's Not

| Component | Status | Reason |
|-----------|--------|--------|
| Store data as felts | **REQUIRED** | STARK mathematical constraint |
| Convert byte→felt queries | **REQUIRED** | Cairo can only read felts |
| Read felts in Cairo | **REQUIRED** | Only way to access Merkle tree data |
| Extract bytes in Rust | **REQUIRED** | 1,600x more efficient than Cairo |
| Hash felt indices | **OPTIONAL** | Verification aid, could be simplified |

## Summary

1. **STARKs work with field elements** - This is mathematical bedrock, not a design choice
2. **Transaction data is stored as felts** - Pre-converted when building Merkle tree
3. **Cairo reads felts, not bytes** - The only operation Cairo can perform
4. **Byte extraction happens in Rust** - Massively more efficient outside the proof
5. **Felt conversion is mandatory** - The only question is how to verify it

The felt→byte workflow isn't an optimization—it's the only way to build a STARK-based transaction proof system.

## Further Reading

- [StarkWare STARK Explainer](https://starkware.co/stark/) - Mathematical foundation
- [Cairo Documentation](https://www.cairo-lang.org/docs/) - Memory model and constraints
- [Finite Field Arithmetic](https://en.wikipedia.org/wiki/Finite_field) - Why field elements
- `cairo/scripts/verify_merkle_proof.cairo` - The Cairo program implementation
- `docs/architecture/QUERY_HASH_SIMPLIFICATION.md` - Potential verification optimizations
