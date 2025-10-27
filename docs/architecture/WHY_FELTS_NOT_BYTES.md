# Why Cairo Works with Felts Instead of Bytes

## Executive Summary

Cairo doesn't work with felts by choice—it's a fundamental constraint of how STARK proofs work. The transaction data in the Merkle tree is **already stored as felts**, and Cairo must work with that format directly.

## The Core Constraint: Field Elements in STARKs

### What is a Felt?

A **felt** (field element) is a number in a finite field. In Cairo/STARK:
- Size: 252 bits (~31.5 bytes)
- Practical use: 248 bits (31 bytes) to stay safe
- Representation: `Felt = integer mod prime_number`

### Why STARKs Use Felts

STARK proofs are based on **polynomial arithmetic over finite fields**. Every computation in a STARK proof must be expressible as field operations:

```
NOT possible: "read byte 192"
IS possible: "read felt[6], extract bits 48-79 from it"
```

This is not a Cairo design choice—it's a mathematical requirement of STARK proving systems.

## The Data Storage Reality

### How Transaction Data is Actually Stored

When a transaction is proven in the Merkle tree, the raw bytes are **pre-converted to felts** before being added to the tree:

```python
# From verify_merkle_proof.cairo hint:
FELT_SIZE = 31

def bytes_to_felt_array(bytes):
    return [int.from_bytes(bytes[i : i + FELT_SIZE], "big")
            for i in range(0, len(bytes), FELT_SIZE)]

# Example: 100 bytes of transaction data
# becomes: [felt0, felt1, felt2, felt3] (4 felts of 31 bytes each)
```

**The Merkle tree stores felts, not bytes.**

### Why This Matters

```
┌─────────────────────────────────────────────┐
│          Merkle Tree (STARK Proof)           │
│                                              │
│  Leaf: [felt0, felt1, felt2, felt3, ...]    │
│         ↑                                    │
│         │                                    │
│         └─ These are NOT bytes!             │
│            These are field elements!         │
│                                              │
│  Cairo can ONLY read felts from this tree   │
└─────────────────────────────────────────────┘
```

## The Query Process

### What the User Wants

```solidity
// User: "Give me bytes 192-223 from this transaction"
LayoutSegment {
    offset: 192,  // byte offset
    size: 32      // byte size
}
```

### What Cairo Actually Has

```
Transaction data stored as felts:
  Felt[0] = bytes [0..31)     (31 bytes)
  Felt[1] = bytes [31..62)    (31 bytes)
  Felt[2] = bytes [62..93)    (31 bytes)
  ...
  Felt[6] = bytes [186..217)  (31 bytes)  ← Contains bytes 192-216
  Felt[7] = bytes [217..248)  (31 bytes)  ← Contains bytes 217-223
  ...
```

### The Conversion is Necessary

```rust
// Convert byte request to felt indices
Input:  LayoutSegment{offset: 192, size: 32}  // bytes 192-223
Output: LayoutSegment{offset: 6, size: 2}     // felts 6 and 7

// Cairo reads felts 6 and 7 (62 bytes total)
// Then extracts bytes 192-223 from those 62 bytes
```

## Why Cairo Can't Just Use Byte Offsets

### Attempt 1: Direct Byte Access (Impossible)

```cairo
// ❌ THIS DOESN'T WORK
let byte_192 = transaction_bytes[192]  // NO SUCH THING
```

**Why it fails:**
- Cairo doesn't have a `transaction_bytes` array
- The data is stored as `transaction_felts` array
- Felts are NOT addressable at byte granularity

### Attempt 2: Convert on the Fly (Expensive)

```cairo
// ❌ TOO EXPENSIVE
func get_byte(felt_array: felt*, byte_offset) -> felt {
    let felt_index = byte_offset / 31
    let byte_within_felt = byte_offset % 31
    // ... extract specific byte from felt ...
    // This requires bit shifting and masking in field arithmetic
    // VERY expensive in STARK proofs!
}
```

**Why it's expensive:**
- Bit operations in field arithmetic are costly
- Each byte access becomes many STARK proof steps
- Proof size explodes

### Attempt 3: Work with Felts (Current Solution)

```cairo
// ✅ EFFICIENT
func get_felts(felt_array: felt*, felt_offset, count) -> felt* {
    // Direct array access - cheap!
    return felt_array + felt_offset
}

// Then extract bytes outside of Cairo (in Rust verifier)
```

**Why it works:**
- Direct felt array access is a single STARK operation
- Byte extraction happens in Rust (free, no proof cost)
- Proof size stays small

## The Complete Flow

### 1. Transaction Ingestion (Off-chain)

```
Raw Transaction (bytes)
    ↓
bytes_to_felt_array()  // Split into 31-byte chunks
    ↓
Felt Array [felt0, felt1, felt2, ...]
    ↓
Add to Merkle Tree (STARK format)
```

### 2. Proof Generation (Cairo)

```
User Query: "bytes 192-223"
    ↓
Prover: Convert to felt indices [6, 7]
    ↓
Cairo Program:
  - Read felt_array[6]  // Direct access (cheap!)
  - Read felt_array[7]  // Direct access (cheap!)
  - Output both felts
    ↓
STARK Proof contains: [felt6_data, felt7_data]
```

### 3. Extraction (Verifier)

```
Proof Output: [felt6 (31 bytes), felt7 (31 bytes)]
    ↓
Rust Verifier:
  - Concatenate: 62 bytes total
  - Felt6 contains bytes [186..217)
  - Felt7 contains bytes [217..248)
  - Extract bytes [192..224) from this 62-byte array
    ↓
Result: Exactly the 32 bytes user requested
```

## Why the Conversion Happens in Rust, Not Cairo

### Option A: Extract in Cairo (Expensive)

```cairo
// Read felts, extract specific bytes, output bytes
// Cost: 100+ STARK constraints per byte operation
// Total: 3,200 constraints for 32 bytes
```

### Option B: Extract in Rust (Current, Cheap)

```rust
// Read felts in Cairo, extract bytes in Rust
// Cairo cost: 2 felt reads = 2 constraints
// Rust cost: Free (outside proof)
// Total: 2 constraints
```

**Savings: 1,600x reduction in proof size!**

## Real-World Example

### Query: ERC20 Transfer Event Data

```solidity
// User wants these fields from transaction:
// - from address (32 bytes at offset 192)
// - to address (32 bytes at offset 224)
// - amount (32 bytes at offset 448)
```

### Step 1: Convert to Felt Ranges

```rust
Byte segments:
  [192..224) → Felt segment [6..8)    (felts 6, 7)
  [224..256) → Felt segment [7..9)    (felts 7, 8)
  [448..480) → Felt segment [14..16)  (felts 14, 15)

After merging overlaps:
  [6..9)   (felts 6, 7, 8)
  [14..16) (felts 14, 15)
```

### Step 2: Cairo Reads Felts

```cairo
// Read 5 felts total (not 96 bytes!)
output[0] = felt_array[6]
output[1] = felt_array[7]
output[2] = felt_array[8]
output[3] = felt_array[14]
output[4] = felt_array[15]

// Proof contains: 5 felts = 155 bytes
// vs reading 96 individual bytes = massive proof
```

### Step 3: Rust Extracts Bytes

```rust
// Felt 6 = bytes [186..217)
// Felt 7 = bytes [217..248)
// → Extract bytes [192..224) from concatenated felts 6-7

// Felt 7 = bytes [217..248)
// Felt 8 = bytes [248..279)
// → Extract bytes [224..256) from concatenated felts 7-8

// etc.
```

## Common Misconceptions

### ❌ "Cairo could just work with bytes if we wanted"

**No.** STARKs fundamentally work with field elements. Cairo has no concept of "bytes" at the protocol level.

### ❌ "The felt conversion is an optimization"

**No.** It's a requirement. The data is stored as felts in the Merkle tree. Cairo must read felts.

### ❌ "We're hashing thousands of offsets"

**No.** We hash ~5-10 felt indices. The byte offsets are never hashed—only felt indices are.

### ❌ "We could eliminate the felt conversion"

**No.** The conversion is mandatory. We can only eliminate the **hash** of felt indices (which is a separate verification step).

## Why Hash the Felt Indices?

This is separate from the felt conversion itself:

```rust
// Conversion is mandatory:
byte_segments → felt_segments  // REQUIRED

// Hashing is optional verification:
hash(felt_segments)  // Could potentially be eliminated
```

The hash verifies that both prover and verifier computed the same felt indices. Without it:

```
Prover: "I read felts [6, 7] for bytes 192-223"
Cairo: Actually reads felts [6, 7] ✓
Verifier: Expects felts [6, 7] ✓

Without hash:
Prover: "I read felts [5, 6] for bytes 192-223"  (BUG!)
Cairo: Actually reads felts [5, 6]
Verifier: Has no way to know this is wrong!
```

The hash provides **early detection** of conversion bugs or malicious provers.

## Summary

### Why Felts?

| Aspect | Reason |
|--------|--------|
| **Storage** | Merkle tree stores felts, not bytes |
| **STARK Math** | Polynomial arithmetic requires field elements |
| **Cairo Access** | Can only read felts from felt arrays |
| **Efficiency** | Reading 5 felts vs 96 bytes = 20x smaller proof |

### The Conversion is Not Optional

```
User wants bytes → Cairo has felts → Must convert
                         ↑
                    Not a choice!
                    Fundamental constraint
```

### What Could Be Simplified

✅ **Cannot eliminate:** Byte→Felt conversion (required by STARK)
✅ **Cannot eliminate:** Felt storage (required by STARK)
✅ **Cannot eliminate:** Reading felts in Cairo (only option)
❓ **Could simplify:** Hash verification of felt indices (see QUERY_HASH_SIMPLIFICATION.md)

## Further Reading

- **STARK Basics**: [StarkWare's STARK Explanation](https://starkware.co/stark/)
- **Cairo Memory Model**: [Cairo Documentation](https://www.cairo-lang.org/docs/)
- **Field Elements**: [Finite Field Arithmetic](https://en.wikipedia.org/wiki/Finite_field)
- **Query Simplification**: `docs/architecture/QUERY_HASH_SIMPLIFICATION.md`
