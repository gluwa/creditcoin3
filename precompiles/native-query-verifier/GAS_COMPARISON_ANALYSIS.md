# Gas Cost Analysis: Native Precompile vs Solidity Contract

## Current Precompile Gas Costs

```solidity
// Current costs in the precompile
GAS_BASE_VERIFY: 35,000         // Base overhead
GAS_PER_TX_BYTE: 16             // Per byte of transaction data
GAS_PER_SIBLING: 3,000           // Per Merkle sibling verification
GAS_PER_CONTINUITY_BLOCK: 5,000 // Per continuity block
GAS_STORAGE_LOOKUP: 2,600        // Per storage read
```

## Solidity Contract Gas Costs (Estimated)

### 1. Merkle Proof Verification in Solidity

```solidity
function verifyMerkleProof(
    bytes memory txData,
    bytes32[] memory siblings,
    bytes32 root,
    uint256 index
) public pure returns (bool) {
    // Gas breakdown:
    // - Function call overhead: ~21,000 (base transaction)
    // - Memory operations: ~3 gas per word
    // - Keccak256 hash: ~30 + 6 per word
    
    bytes32 currentHash = keccak256(abi.encodePacked(uint8(0x00), txData));
    // Cost: ~1,500 gas for average 200-byte tx
    
    for (uint i = 0; i < siblings.length; i++) {
        // Each iteration:
        // - Memory read: ~3 gas
        // - Comparison: ~3 gas
        // - Keccak256: ~150 gas (for 64 bytes)
        // - Conditional logic: ~10 gas
        // Total per sibling: ~166 gas
        
        if (index % 2 == 0) {
            currentHash = keccak256(abi.encodePacked(uint8(0x01), currentHash, siblings[i]));
        } else {
            currentHash = keccak256(abi.encodePacked(uint8(0x01), siblings[i], currentHash));
        }
        index /= 2;
    }
    
    return currentHash == root;
}
```

**Solidity Merkle Verification Costs:**
- Base function call: 21,000 gas
- Initial leaf hash (200 bytes): ~1,500 gas
- Per sibling verification: ~166 gas
- Total for 10 siblings: ~23,160 gas

**Precompile Merkle Verification Costs:**
- Base: 35,000 gas (includes all overhead)
- Transaction data (200 bytes): 3,200 gas (16 * 200)
- Per sibling (10 siblings): 30,000 gas (3,000 * 10)
- Total: 68,200 gas

### 2. Continuity Chain Verification in Solidity

```solidity
function verifyContinuityChain(
    Block[] memory blocks,
    uint256 chainId
) public view returns (bool) {
    // Each block verification:
    // - SLOAD for stored digest: 2,100 gas (warm) or 2,600 (cold)
    // - Keccak256 for digest: ~200 gas
    // - Comparisons: ~10 gas
    // Total per block: ~2,810 gas
    
    for (uint i = 0; i < blocks.length; i++) {
        bytes32 storedDigest = lastDigests[chainId][blocks[i].blockNumber];
        // SLOAD: 2,600 gas (cold)
        
        bytes32 computedDigest = keccak256(abi.encode(blocks[i]));
        // Keccak256: ~200 gas
        
        if (storedDigest != computedDigest) return false;
        // Comparison: ~3 gas
    }
    
    return true;
}
```

**Solidity Continuity Costs:**
- Per block: ~2,810 gas
- For 10 blocks: ~28,100 gas

**Precompile Continuity Costs:**
- Per block: 5,000 + 2,600 = 7,600 gas
- For 10 blocks: 76,000 gas

### 3. Data Extraction in Solidity

```solidity
function extractData(
    bytes memory txData,
    LayoutSegment[] memory segments
) public pure returns (bytes[] memory) {
    // Per segment extraction:
    // - Memory allocation: ~100 gas
    // - Slice operation: ~50 gas
    // - Bounds checking: ~20 gas
    // Total per segment: ~170 gas
    
    bytes[] memory results = new bytes[](segments.length);
    
    for (uint i = 0; i < segments.length; i++) {
        // Memory slice and copy
        results[i] = slice(txData, segments[i].offset, segments[i].length);
    }
    
    return results;
}
```

**Solidity Extraction Costs:**
- Per segment: ~170 gas
- For 4 segments: ~680 gas

**Precompile Extraction Costs:**
- Included in base cost

## Recommended Adjustments for Realistic Gas Costs

### Current vs Recommended Gas Costs

| Operation | Current | Solidity Estimate | Recommended | Rationale |
|-----------|---------|------------------|-------------|-----------|
| Base Verify | 35,000 | 21,000 | **21,000** | Match base transaction cost |
| Per TX Byte | 16 | 16 | **16** | Correct - matches calldata |
| Per Sibling | 3,000 | 166 | **200** | Precompile efficiency gain |
| Per Continuity Block | 5,000 | 2,810 | **3,000** | Slight premium for verification |
| Storage Lookup | 2,600 | 2,600 | **2,600** | Correct - matches SLOAD |

### Proposed New Constants

```rust
// More realistic gas costs based on Solidity comparison
const GAS_BASE_VERIFY: u64 = 21_000;      // Match base transaction cost
const GAS_PER_TX_BYTE: u64 = 16;          // Correct - matches calldata
const GAS_PER_SIBLING: u64 = 200;         // Efficient native hashing
const GAS_PER_CONTINUITY_BLOCK: u64 = 3_000; // Storage + verification
const GAS_STORAGE_LOOKUP: u64 = 2_600;    // Correct - matches cold SLOAD
```

## Why Precompiles Are More Efficient

### 1. **Native Execution**
- Solidity runs in EVM interpreter (10-100x overhead)
- Precompiles run in native Rust code (direct CPU execution)

### 2. **Memory Management**
- Solidity: EVM memory expansion costs (quadratic)
- Precompile: Direct memory access (linear)

### 3. **Hashing Performance**
- Solidity keccak256: ~30 + 6 per word
- Native keccak256: ~10x faster in practice

### 4. **No Stack/Memory Overhead**
- Solidity: Stack operations, memory copies
- Precompile: Direct register operations

## Example Comparison: Typical Query

### Scenario: ERC20 Transfer Verification
- 200-byte transaction
- 10 siblings (5-level tree)
- 5 continuity blocks

### Solidity Total:
```
Base:           21,000
TX processing:   3,200  (200 * 16)
Merkle verify:   1,660  (10 * 166)
Continuity:     14,050  (5 * 2,810)
Data extract:      680  (4 * 170)
-----------------------
Total:          40,590 gas
```

### Current Precompile:
```
Base:           35,000
TX processing:   3,200  (200 * 16)
Merkle verify:  30,000  (10 * 3,000)
Continuity:     38,000  (5 * 7,600)
-----------------------
Total:         106,200 gas (2.6x more expensive!)
```

### Recommended Precompile:
```
Base:           21,000
TX processing:   3,200  (200 * 16)
Merkle verify:   2,000  (10 * 200)
Continuity:     15,000  (5 * 3,000)
-----------------------
Total:          41,200 gas (similar to Solidity)
```

## Conclusion

The current precompile gas costs are **significantly overpriced** compared to what a Solidity implementation would cost. The precompile should be:

1. **Cheaper than Solidity** due to native execution efficiency
2. **But not too cheap** to prevent DoS attacks
3. **Competitive** to incentivize usage

### Recommended Actions:
1. Reduce `GAS_BASE_VERIFY` from 35,000 to 21,000
2. Reduce `GAS_PER_SIBLING` from 3,000 to 200
3. Reduce `GAS_PER_CONTINUITY_BLOCK` from 5,000 to 3,000
4. Keep `GAS_PER_TX_BYTE` and `GAS_STORAGE_LOOKUP` as-is

This would make the precompile approximately **equal or slightly cheaper** than Solidity, which is appropriate given the native execution advantages while still preventing abuse.
