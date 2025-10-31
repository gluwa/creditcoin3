# Native Query Verification Flow Visualization

## Overview
The native query verification system provides cryptographic proof that specific data exists in a blockchain transaction through a two-layer verification process.

## Complete Verification Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        USER SUBMITS QUERY REQUEST                            │
│  • Chain ID: 2                                                               │
│  • Block Height: 736                                                         │
│  • Transaction Index: 0                                                      │
│  • Data Segments: [offset:479,size:32], [offset:223,size:32], ...          │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          STEP 1: MERKLE PROOF VERIFICATION                   │
│                     "Is this transaction in the block?"                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│   Transaction Data (991 bytes)                    Block #736                 │
│   ┌──────────────────────┐                       ┌────────────┐             │
│   │ 0x00000000000000... │                       │   Tx 0     │◄──You are    │
│   │ [actual tx bytes]    │                       │   Tx 1     │   here      │
│   └──────────────────────┘                       │   Tx 2     │             │
│            │                                      │   ...      │             │
│            │                                      └────────────┘             │
│            ▼                                           │                     │
│   Hash with Pedersen                                   │                     │
│   (prepend 0x00 for leaf)                             ▼                     │
│            │                                    Build Merkle Tree            │
│            ▼                                    ┌─────────────┐              │
│   Leaf Hash: 0xABCD...                        /               \             │
│            │                                  /                 \            │
│            │                            ┌────┴───┐         ┌────┴───┐       │
│            │                           /         \        /         \       │
│            │                          Tx0       Tx1      Tx2       Tx3      │
│            │                           ▲                                     │
│            └───────────────────────────┘                                     │
│                                                                               │
│   Merkle Proof Siblings: []  (empty for single tx)                          │
│   Expected Root: 0x06c08983896b5524016380e290e475d539a82ef13f86ae27...      │
│   Computed Root: 0x06c08983896b5524016380e290e475d539a82ef13f86ae27...      │
│                                                                               │
│   ✅ MERKLE PROOF VALID - Transaction IS in block #736 at index 0           │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    STEP 2: CONTINUITY CHAIN VERIFICATION                     │
│                  "Is this block part of the attested chain?"                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│   Attestations from Creditcoin3:                                             │
│   ┌──────────────┐                              ┌──────────────┐            │
│   │ Block #730   │                              │ Block #740   │            │
│   │ Digest: 0x.. │◄─────────────────────────────│ Digest: 0x.. │            │
│   │ (Attested)   │                              │ (Attested)   │            │
│   └──────────────┘                              └──────────────┘            │
│                                                                               │
│   Build Continuity Chain (fetch actual blocks):                             │
│                                                                               │
│   Block #731 ──► Block #732 ──► Block #733 ──► ... ──► Block #736           │
│       │              │              │                       │                │
│       ▼              ▼              ▼                       ▼                │
│   prev_digest    prev_digest    prev_digest            prev_digest          │
│   = attest_730   = hash_731     = hash_732             = hash_735           │
│       │              │              │                       │                │
│       ▼              ▼              ▼                       ▼                │
│    digest =       digest =       digest =               digest =            │
│    hash_731       hash_732       hash_733               hash_736            │
│                                                                               │
│   Each digest = pedersen_hash(block_number, root, prev_digest)              │
│                                                                               │
│   ✅ CONTINUITY CHAIN VALID - Block #736 is connected to attestation        │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                        STEP 3: DATA EXTRACTION                               │
│                   "Extract the requested data segments"                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│   Transaction Data (verified):                                               │
│   ┌─────────────────────────────────────────────────────────────────┐       │
│   │ [0-478] | [479-510] | [223-254] | [255-286] | [287-318] | ...   │       │
│   └─────────────────────────────────────────────────────────────────┘       │
│              ▲          ▲           ▲           ▲                            │
│              │          │           │           │                            │
│   Extract:   │          │           │           │                            │
│              │          │           │           │                            │
│   Segment 0: └──────────┘           │           │  (nonce)                  │
│   Segment 1:           └────────────┘           │  (from address)           │
│   Segment 2:                       └────────────┘  (to address)             │
│   Segment 3:                                   └─  (value)                  │
│                                                                               │
│   Result Segments:                                                           │
│   • [479,32]: 0x0000...0001 (nonce = 1)                                    │
│   • [223,32]: 0x0000...f39fd6e51aad88f6f4ce6ab8827279cfffb92266 (from)     │
│   • [255,32]: 0x0000...87a879362d637e087d8774f111468a7ed14db702 (to)       │
│   • [287,32]: 0x0000...03e493fcd86d6898 (value)                            │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           FINAL VERIFICATION RESULT                          │
│                                                                               │
│   Status: SUCCESS (0)                                                        │
│   Proof Guarantees:                                                          │
│   ✓ Transaction data is exactly as it was in block #736 at index 0         │
│   ✓ Block #736 is part of the attested chain                               │
│   ✓ Extracted data segments are cryptographically verified                  │
│                                                                               │
│   This provides trustless proof that the queried data exists on-chain       │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Error Cases

### Case 1: Block Without Attestations (e.g., Block #4)
```
Block #4 Query
     │
     ▼
No attestations found for blocks 0-9
     │
     ▼
❌ "Continuity chain cannot be empty"
(Cannot prove block is part of attested chain)
```

### Case 2: Invalid Merkle Proof
```
Wrong transaction data provided
     │
     ▼
Computed root ≠ Expected root
     │
     ▼
❌ Status: 1 (MerkleProofInvalid)
```

### Case 3: Broken Continuity Chain
```
Block #735 missing or incorrect digest
     │
     ▼
prev_digest of #736 ≠ digest of #735
     │
     ▼
❌ Status: 2 (ContinuityChainInvalid)
```

## Key Components

### Merkle Tree (Starknet Pedersen MMR)
- **Purpose**: Proves transaction inclusion in a block
- **Structure**: Binary tree with Pedersen hash
- **Leaf nodes**: Hash of (0x00 || transaction_data)
- **Inner nodes**: Hash of (0x01 || left_child || right_child)
- **Proof**: Siblings along the path from leaf to root

### Continuity Chain
- **Purpose**: Proves block is part of the attested chain
- **Structure**: Linked list of blocks via digest chain
- **Digest formula**: `pedersen_hash(block_number, merkle_root, prev_digest)`
- **Anchors**: Attestations from validators stored on Creditcoin3

### Data Extraction
- **Purpose**: Extract specific data segments from verified transaction
- **Input**: List of (offset, size) pairs
- **Output**: Extracted bytes at each location
- **Guarantee**: Data comes from cryptographically verified source

## Security Properties

1. **Tamper-proof**: Any modification to transaction data changes the Merkle root
2. **Chain integrity**: Continuity chain ensures no blocks can be inserted/removed
3. **Attestation anchoring**: Chain is anchored to validator-signed attestations
4. **Minimal trust**: Only need to trust the attestation validators, not data providers
5. **Efficient verification**: O(log n) for Merkle proof, O(m) for continuity chain

## Gas Costs

- **Merkle verification**: ~50,000 gas base + 10,000 per sibling
- **Continuity verification**: ~20,000 gas per block in chain
- **Data extraction**: ~5,000 gas per segment
- **Total for typical query**: ~200,000 - 500,000 gas
