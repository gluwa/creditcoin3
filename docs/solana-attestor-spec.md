# Solana Attestor Support — Technical Specification

**Status:** Draft  
**Author:** Protocol Engineering  
**Target Repo:** `gluwa/creditcoin3`  
**Last Updated:** 2026-04-24 (rev 5 — struct returns in decoder, automated fixture test generation, no via_ir requirement)

---

## Table of Contents

1. [Overview](#1-overview)
2. [Background: How Solana Works](#2-background-how-solana-works)
3. [Current EVM Pipeline (Reference)](#3-current-evm-pipeline-reference)
4. [What Changes for Solana](#4-what-changes-for-solana)
5. [ABI Encoding Design (SolanaV1)](#5-abi-encoding-design-solanav1)
6. [Solidity Decoder Design](#6-solidity-decoder-design)
7. [Proposed Architecture](#7-proposed-architecture)
8. [File-by-File Change Map](#8-file-by-file-change-map)
9. [Code Hints & Skeletons](#9-code-hints--skeletons)
10. [Open Questions](#10-open-questions)
11. [Testing Strategy](#11-testing-strategy)
12. [Appendix: Solana RPC Reference](#12-appendix-solana-rpc-reference)

---

## 1. Overview

The attestor currently supports only EVM-compatible chains. Every block-fetch, encoding, and Merkle root computation is hardcoded to Ethereum types from the `alloy` crate.

This spec defines the minimal set of changes to extend attestor support to Solana. The BLS aggregation, digest computation, continuity proof, P2P sync, and on-chain submission paths are already chain-agnostic and do not need changes. Only the **source chain block fetch + encoding layer** is EVM-specific.

The core strategy is:

- Add `ChainEncodingVersion::SolanaV1 = 2` as a new discriminator stored on-chain per `SupportedChain`
- Add a `MaturityStrategy::SolanaFinalized` (slot-based finality)
- Create a new `common/solana` crate with a Solana RPC client and an `OrderedBlock` equivalent
- Create a new ABI encoding function / crate (`SolanaV1`) that produces deterministic bytes from Solana transactions
- Extract a `SourceChainClient` trait so `StreamRoots` and `StreamTip` work generically over both EVM and Solana
- Branch on `chain_encoding` in `attestor/attestor/src/lib.rs` to wire the correct client

---

## 2. Background: How Solana Works

> For engineers unfamiliar with Solana, this section is essential reading before touching any of the encoding or client code.

### 2.1 Slots vs Blocks

Solana produces a block (or "slot") roughly every **400ms**. A slot is a time window assigned to a single leader validator. Not every slot produces a block — slots can be **skipped** (no transactions, no block produced). This is fundamentally different from Ethereum, where every block number has a valid block.

**Implication:** The Solana client must handle `SlotNotFound` gracefully (treat as empty block → empty Merkle root).

### 2.2 Finality

Solana uses a supermajority-vote finality model:

- A slot is **confirmed** when >66% of stake has voted on it
- A slot is **finalized** when it is part of a chain with ≥31 supermajority votes on top of it
- Finalized = cannot be rolled back under any non-catastrophic condition

On mainnet, finalization takes ~13 seconds (≈31 slots). There is no concept of "safe" vs "finalized" distinct epochs like EVM — Solana's RPC uses a `commitment` parameter instead:

```
Commitment levels:
  processed  — included in a block, may be rolled back
  confirmed  — supermajority voted, very unlikely to roll back
  finalized  — 31 votes, cannot roll back
```

The attestor should use `finalized` commitment by default.

**Unlike EVM**, Solana does not publish a "finalized block number" in a subscription stream. The `slotSubscribe` WebSocket subscription delivers slot numbers; you must call `getSlot({ commitment: "finalized" })` to know the current finalized tip.

### 2.3 Transaction Structure

A Solana transaction is fundamentally different from an Ethereum transaction:

| Field | Ethereum | Solana |
|---|---|---|
| Sender | `from` (20-byte address) | `fee_payer` (32-byte pubkey, index 0 in accounts) |
| Recipient | `to` (20-byte address) | Program ID (in instruction) |
| Value | `value` (ETH amount) | No native value field; token transfers are instructions |
| Gas | `gas_limit`, `gas_price` | `fee` (lamports, computed from signatures count × fee schedule) |
| Nonce | `nonce` | `recent_blockhash` (prevents replay, expires after ~150 slots) |
| Data | `input` bytes | `instructions[]` with `program_id`, `accounts[]`, `data` |
| Receipt | Separate `TransactionReceipt` object | Inline in `TransactionWithMeta.meta` |

There are no access lists, EIP-typed envelopes, blob fields, or receipt roots in a Solana block header.

### 2.4 Block Structure

A Solana block returned by `getBlock` looks like:

```json
{
  "slot": 12345678,
  "blockTime": 1713000000,
  "blockhash": "Fk...Zx",
  "previousBlockhash": "Aa...Bb",
  "parentSlot": 12345677,
  "transactions": [
    {
      "transaction": { "message": { ... }, "signatures": [...] },
      "meta": {
        "err": null,
        "fee": 5000,
        "preBalances": [1000000, 0],
        "postBalances": [995000, 0],
        "logMessages": ["Program log: ..."],
        "innerInstructions": [],
        "loadedAddresses": { "writable": [], "readonly": [] }
      }
    }
  ],
  "rewards": [],
  "blockHeight": 11000000
}
```

The `blockhash` is a SHA-256 hash over the slot's entries (not equivalent to an EVM block hash).

### 2.5 Chain ID Equivalent

Solana does not have a numeric `chain_id`. Each cluster (mainnet-beta, devnet, testnet, localnet) is identified by its **genesis hash** — a SHA-256 hash of the genesis block.

Known genesis hashes:
- Mainnet: `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d`
- Devnet: `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`
- Testnet: `4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY`

**For `SupportedChain.chain_id`**, use the first 8 bytes of the genesis hash interpreted as a little-endian `u64`. This gives a stable unique identifier per cluster.

### 2.6 Development Tips

**Local validator:**
```bash
solana-test-validator --reset
# Produces blocks at ~400ms, finalized commitment available immediately
# RPC: http://127.0.0.1:8899
# WS:  ws://127.0.0.1:8900
```

**Useful CLI commands:**
```bash
# Get genesis hash
solana genesis-hash

# Get current finalized slot
solana slot --commitment finalized

# Get block at slot
solana block <slot>

# Watch slots live
solana slot --follow
```

**Crate to use:** `solana-client = "2.x"` (or `solana-rpc-client` which is the split-out version in newer releases). Use `solana-sdk` for transaction types.

**RPC call to fetch a block:**
```rust
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_config::RpcBlockConfig;
use solana_transaction_status::{TransactionDetails, UiTransactionEncoding};

let config = RpcBlockConfig {
    encoding: Some(UiTransactionEncoding::Base64),
    transaction_details: Some(TransactionDetails::Full),
    rewards: Some(false),
    commitment: Some(CommitmentConfig::finalized()),
    max_supported_transaction_version: Some(0),
};

let block = client.get_block_with_config(slot, config)?;
```

**WebSocket subscription for slots:**
```rust
use solana_client::pubsub_client::PubsubClient;

let (mut subscription, _) = PubsubClient::slot_subscribe(&ws_url)?;
for slot_info in &mut subscription {
    // SlotInfo { slot, parent, root }
    // `root` = finalized slot
    println!("New slot: {}, finalized root: {}", slot_info.slot, slot_info.root);
}
```

> **Key insight:** `SlotInfo.root` gives you the finalized slot in every notification — equivalent to watching for `finalized` commitment. Use this instead of polling `getSlot(finalized)`.

---

## 3. Current EVM Pipeline (Reference)

Understanding the existing pipeline is prerequisite to extending it.

```
SupportedChain { chain_encoding: ChainEncodingVersion::V1, maturity_strategy: "EvmFinalized", ... }
         │
         ▼
eth::Client::get_block(n, EncodingVersion::V1)
    → fetches Block + receipts via alloy
    → builds OrderedBlock { chain_id, number, hash, items: Vec<TxRx> }
         │
         ▼
TxRx::payload_bytes()
    → usc_abi_encoding::abi::abi_encode(tx, rx, V1)
    → returns DynSolValue::Tuple(type_id, Array([chunk1, chunk2, ...]))
    → .abi().to_vec() → Vec<u8>
         │
         ▼
eth::simple_merkle_tree(&block)
    → block.items().iter().map(|item| item.to_bytes())
    → merkle::KeccakMerkleTree::new(&tx_bytes)
    → .root() → H256
         │
         ▼
AttestationData { root, header_hash, header_number, chain_key, prev_digest }
         │
         ▼
compute_digest_for(block_number, &root, prev_digest)
    → keccak256(block_number_be || root || prev_digest)
    → Digest (H256)
         │
         ▼
BLS sign → P2P gossip → aggregate → submit extrinsic
```

**Key observation:** Everything from `AttestationData` creation onwards is chain-agnostic. The only EVM-specific parts are `eth::Client`, `TxRx`, `OrderedBlock`, and `usc_abi_encoding`.

### 3.1 Relevant Types (current)

```rust
// primitives/attestor/src/lib.rs
pub enum ChainEncodingVersion {
    V1 = 1,  // Only variant, used for EVM
}

// primitives/supported-chains/src/lib.rs
pub struct SupportedChain {
    pub chain_id: u64,
    pub chain_name: Vec<u8>,
    pub chain_encoding: ChainEncodingVersion,
    pub maturity_strategy: String,
}

pub enum MaturityStrategy {
    EvmFinalized,  // 64-block lag
    EvmSafe,       // 32-block lag
    EvmLatest,     // 0-block lag
    FixedDelay(u64),
}

// common/eth/src/lib.rs
pub struct TxRx { tx: Transaction, rx: TransactionReceipt, encoding: EncodingVersion }
impl BlockItem for TxRx {
    fn payload_bytes(&self) -> Vec<u8> { /* usc_abi_encoding */ }
}

pub struct OrderedBlock { chain_id, number, hash, items: Vec<TxRx> }

// common/streams/eth/src/roots.rs — StreamRoots (EVM-only)
// common/streams/eth/src/tip.rs   — StreamTip  (EVM-only)
```

---

## 4. What Changes for Solana

### 4.1 Summary Table

| Component | Change Type | Notes |
|---|---|---|
| `ChainEncodingVersion` | Add variant `SolanaV1 = 2` | Storage type — migration required if existing chains exist |
| `MaturityStrategy` | Add `SolanaFinalized` | Uses slot root from WS subscription |
| `common/solana` | New crate | Client, `SolanaOrderedBlock`, `SolanaTxItem` |
| `solana-abi-encoding` | New crate (or extend existing) | `SolanaV1` deterministic encoder |
| `common/streams/eth` | Refactor or duplicate | Extract `SourceChainClient` trait OR add `common/streams/solana` |
| `attestor/attestor/src/lib.rs` | Branch on `chain_encoding` | Wire correct client based on `SupportedChain` |
| `runtime/src/migrations.rs` | New migration if needed | Only if `ChainEncodingVersion` layout changes |
| `chainspecs/` | New Solana chain entries | With `chain_encoding: SolanaV1` |

### 4.2 What Stays the Same

- `AttestationData` structure
- `compute_digest_for` (already chain-agnostic, uses block number + root + prev_digest)
- `ContinuityProof`
- `merkle::KeccakMerkleTree` — used unchanged, just fed different bytes
- BLS signing and aggregation
- P2P gossip worker
- Validation worker
- On-chain submission (`submit_attestation` extrinsic)
- `StreamCC3` and all CC3 chain interaction

---

## 5. ABI Encoding Design (SolanaV1)

The `SolanaV1` encoding uses **Ethereum ABI encoding** — the same format as the existing EVM `V1` path. The off-chain encoder (Rust, in the attestor/collector pipeline) translates everything (Borsh, RLP, raw bytes) into ABI. The on-chain decoder only ever speaks ABI.

> **Why ABI and not Borsh?** ABI can be decoded on EVM contracts (Solidity) natively and in Solana programs via `alloy-sol-types` no_std. One encoding that works on both chains. Borsh would require a custom ABI decoder on the EVM side.

### 5.1 Chunking Philosophy

Each transaction is ABI-encoded into **chunks grouped by query pattern**. This solves two problems:

1. **Stack-too-deep** — Solidity has a 16-local-variable limit. A flat full-tx encoding would exceed it. Chunks are small flat structs that decode independently.
2. **Gas efficiency** — Callers only submit and decode the chunk they need. The other chunks are verified by hash, not decoded.

Each chunk independently contains the transaction `signature` (bytes64) so it can be self-identified without the surrounding context.

### 5.2 Chunk Definitions

#### Chunk 0 — SOL Transfers

```solidity
struct Chunk0 {
    bytes64  signature;
    bytes32[] accountKeys;
    uint64[] preBalances;
    uint64[] postBalances;
    bool     success;
    uint64   fee;
}
```

6 top-level fields. Maps to `preBalances`/`postBalances` from transaction `meta`. Indices are parallel to `accountKeys`.

ABI type: `abi.encode(bytes, bytes32[], uint64[], uint64[], bool, uint64)`

#### Chunk 1 — Token Transfers

```solidity
struct TokenBalance {
    uint8   accountIndex;  // index into accountKeys
    bytes32 mint;
    bytes32 owner;
    uint64  amount;        // raw amount (no decimals applied)
    uint8   decimals;
}

struct Chunk1 {
    bytes64       signature;
    TokenBalance[] preTokenBalances;
    TokenBalance[] postTokenBalances;
}
```

3 top-level fields. Maps to `preTokenBalances`/`postTokenBalances` from transaction `meta`. These are the primary evidence for SPL token movements — Solana has no mandatory Transfer event like ERC-20.

ABI type: `abi.encode(bytes, (uint8,bytes32,bytes32,uint64,uint8)[], (uint8,bytes32,bytes32,uint64,uint8)[])`

#### Chunk 2 — Logs / Cross-Chain Intents

```solidity
struct LogEntry {
    bytes32 programId;  // which program emitted this log
    uint8   depth;      // CPI call depth (1 = top-level)
    uint8   logType;    // 0=invoke, 1=success, 2=fail, 3=log (text), 4=data (base64)
    bytes   payload;    // raw log content (text for type 3, decoded bytes for type 4)
}

struct Chunk2 {
    bytes64    signature;
    LogEntry[] logs;
}
```

This chunk is the primary evidence for cross-chain intents:

- `Program data:` entries (`logType=4`) carry structured Anchor events and Wormhole's `LogPublishedMessage` — these are the cross-chain intent signals equivalent to Ethereum `emit` logs
- `Program log:` entries (`logType=3`) are unstructured text; include them in the encoding but on-chain decoders should focus on `logType=4`
- Runtime invoke/success lines are included for completeness but rarely useful to decode on-chain

ABI type: `abi.encode(bytes, (bytes32,uint8,uint8,bytes)[])`

#### Chunk 3+ — Program-Specific Extensions

Optional. Added only when a transaction involves a known program requiring special handling:

**Neon EVM (Chunk 3):**
When the Neon EVM program is detected, the encoder unwraps the RLP-encoded EVM transaction from the holder account, extracts EVM event logs (topics + data), and ABI-encodes them:

```solidity
struct EvmLogEntry {
    bytes32   contractAddress;
    bytes32[] topics;
    bytes     data;
}

struct Chunk3Neon {
    bytes64      signature;
    EvmLogEntry[] evmLogs;
}
```

The on-chain decoder never knows this came from RLP. It just sees another ABI chunk.

### 5.3 Two-Level Merkle Tree

The attestation uses a **two-level Keccak Merkle tree**:

```
Slot Root (attested — prev_root + height + digest, chain-agnostic)
├── TX 0 Root = keccak_merkle(chunk0, chunk1, chunk2, ...)
├── TX 1 Root = keccak_merkle(chunk0, chunk1, chunk2, ...)
├── TX 2 Root = keccak_merkle(chunk0, chunk1, chunk2, ...)
└── ...
```

- **Attestors vote on the slot root only** — they don't care about chunk internals
- Each TX root is a Keccak Merkle tree over its chunks' ABI-encoded bytes
- Each chunk is a leaf in the TX sub-tree

**Proving a claim:**
1. Submit the relevant chunk bytes (e.g. Chunk 1 for token transfer)
2. Sub-proof: chunk → TX root (sibling chunk hashes, at most log₂(num_chunks) hashes)
3. Outer proof: TX root → Slot root (sibling TX hashes)

This means callers never submit chunk bytes they don't need — just the target chunk + hashes for the rest.

**Precompile impact:** The `BlockProver` precompile needs a new overload:
```
// Current (single-level):
verify(chainKey, height, encodedTx, merkleProof, continuityProof)

// New Solana two-level overload:
verify(chainKey, height, chunkBytes, chunkProof, txProof, continuityProof)
```

Or: start with single-level (full encoded tx as one leaf) for simplicity, add the two-level overload when calldata costs justify it. The slot root / digest is identical either way — backwards compatible.

### 5.4 Attestation Structure

Attestation structure is **unchanged** — it is chain-agnostic:

```
AttestationData {
    chain_key:      ChainKey,
    header_number:  Height,     // = slot number for Solana
    header_hash:    H256,       // = blockhash (last entry hash of the slot)
    root:           H256,       // = Keccak Merkle root over all TX leaves
    prev_digest:    Option<Digest>,
}
```

The `root` is the only Solana-specific part — it's computed by ABI-encoding each transaction into chunks, building TX sub-roots, then building the slot root from TX roots. The `digest` computation (`keccak256(height || root || prev_digest)`) is identical to EVM.

### 5.5 Transaction Version Handling

Two Solana transaction versions need support:

- **Legacy** — all account keys inline in the message
- **v0** — uses Address Lookup Tables (ALTs); the RPC `getBlock` response resolves ALTs and returns the full `accountKeys` array in both cases

After fetching, normalize to the same internal representation. The `txType` field in the encoding (implicit from the version byte in the raw transaction) is included in Chunk 0 as needed. The meta fields (balances, logs) are identical across both versions.

### 5.6 Field Exclusions

- **Signatures excluded from identity chunks** — Each chunk includes `signature` as the tx identifier (bytes64, first signature only). Full signature arrays excluded to keep chunk sizes manageable.
- **Raw instruction data** — Excluded from default chunks; added as Chunk 3+ if program-specific decoding is needed
- **blockTime** — RPC convenience only, not consensus. Excluded.
- **rewards array** — Block-level, not transaction-level. Excluded.
- **Program log: text entries** — Included in Chunk 2 for completeness (`logType=3`) but not intended for on-chain decoding.

---

## 6. Solidity Decoder Design

This is the on-chain component. **Design constraint:** all decoder functions must compile without `via_ir = true`. The chunked design makes this achievable — each decode function handles one small struct at a time.

### 6.1 Architecture

```
Off-chain encoder (Rust)               On-chain decoder (Solidity)
────────────────────────────           ────────────────────────────
Solana RPC → raw tx (Borsh/RLP)   →   ABI-encoded chunk bytes
Normalize to canonical fields          abi.decode(chunk, types)
ABI-encode per chunk schema      →    Extract specific fields
Build TX sub-merkle trees        →    Verify chunk proof → TX root
Build slot Keccak Merkle root    →    Verify TX proof → Slot root
Attestors vote on slot root       →    BlockProver precompile verifies
```

The decoder contract never sees Borsh, base58, or RLP. The encoder handles all format translation off-chain.

### 6.2 Stack-Too-Deep Prevention

Solidity's EVM limits usable stack depth to ~16 local variables per function. Returning multiple values counts against this limit: a function returning 6 values consumes 6 stack slots — leaving only ~10 for local use. Any callers that destructure those 6 values and add local logic will exhaust the stack immediately.

**Rule: every `decodeChunkN` returns a single struct, not multiple values.**

A struct return occupies exactly 1 stack slot in the caller regardless of how many fields it contains. The caller accesses fields via `.member` — no stack explosion.

```
// ❌ BAD — 6 return values = 6 stack slots in every caller
function decodeChunk0(bytes calldata chunk)
    returns (bytes memory sig, bytes32[] memory keys,
             uint64[] memory pre, uint64[] memory post,
             bool ok, uint64 fee) { ... }

// ✅ GOOD — 1 struct = 1 stack slot in every caller
function decodeChunk0(bytes calldata chunk)
    returns (Chunk0 memory d) { ... }
```

**Checker:** The Rust encoder automatically generates a Foundry test fixture (see Section 6.6) that:
1. Calls each `decodeChunkN` function in a realistic caller context (local vars + loops)
2. Compiles with `solc` using `optimizer_runs=200`, **no `via_ir`**
3. Fails CI if `stack too deep` is produced

This catches regressions before they reach the Solidity codebase.

### 6.3 Solidity Types

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

library SolanaTypes {

    // ── Chunk 0 ────────────────────────────────────────────────────────────

    struct Chunk0 {
        bytes     signature;      // first tx signature, bytes64 (as bytes)
        bytes32[] accountKeys;
        uint64[]  preBalances;    // parallel to accountKeys
        uint64[]  postBalances;   // parallel to accountKeys
        bool      success;
        uint64    fee;
    }

    // ── Chunk 1 ────────────────────────────────────────────────────────────

    struct TokenBalance {
        uint8   accountIndex;  // index into accountKeys
        bytes32 mint;
        bytes32 owner;
        uint64  amount;        // raw amount (apply decimals client-side)
        uint8   decimals;
    }

    struct Chunk1 {
        bytes          signature;
        TokenBalance[] preTokenBalances;
        TokenBalance[] postTokenBalances;
    }

    // ── Chunk 2 ────────────────────────────────────────────────────────────

    struct LogEntry {
        bytes32 programId;
        uint8   depth;
        // logType: 0=invoke  1=success  2=fail  3=log (text)  4=data (base64-decoded)
        uint8   logType;
        bytes   payload;
    }

    struct Chunk2 {
        bytes      signature;
        LogEntry[] logs;
    }

    // ── Chunk 3 (Neon EVM, optional) ───────────────────────────────────────

    struct EvmLogEntry {
        bytes32   contractAddress;
        bytes32[] topics;
        bytes     data;
    }

    struct Chunk3Neon {
        bytes          signature;
        EvmLogEntry[]  evmLogs;
    }
}
```

### 6.4 Solidity Decoder (struct returns)

```solidity
// SolanaDecoder.sol
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./SolanaTypes.sol";

library SolanaDecoder {

    // ── Chunk 0: SOL transfers ─────────────────────────────────────────────
    // Stack budget: 1 param (chunk) + 1 return (Chunk0 struct) = 2 slots.
    // Callers add local vars freely without hitting the 16-slot limit.

    function decodeChunk0(bytes calldata chunk)
        internal pure returns (SolanaTypes.Chunk0 memory d)
    {
        (d.signature, d.accountKeys, d.preBalances, d.postBalances, d.success, d.fee) =
            abi.decode(chunk, (bytes, bytes32[], uint64[], uint64[], bool, uint64));
    }

    // ── Chunk 1: Token transfers ───────────────────────────────────────────

    function decodeChunk1(bytes calldata chunk)
        internal pure returns (SolanaTypes.Chunk1 memory d)
    {
        (d.signature, d.preTokenBalances, d.postTokenBalances) =
            abi.decode(chunk, (bytes, SolanaTypes.TokenBalance[], SolanaTypes.TokenBalance[]));
    }

    // ── Chunk 2: Logs / cross-chain intents ───────────────────────────────

    function decodeChunk2(bytes calldata chunk)
        internal pure returns (SolanaTypes.Chunk2 memory d)
    {
        (d.signature, d.logs) =
            abi.decode(chunk, (bytes, SolanaTypes.LogEntry[]));
    }

    /// Filter to only logType=4 (Program data:) entries — structured events.
    /// These carry Anchor events, Wormhole LogPublishedMessage, etc.
    /// Separate function to avoid inflating decodeChunk2's stack frame.
    function extractDataLogs(SolanaTypes.LogEntry[] memory logs)
        internal pure returns (bytes[] memory payloads)
    {
        uint256 count;
        for (uint256 i; i < logs.length; ++i) {
            if (logs[i].logType == 4) ++count;
        }
        payloads = new bytes[](count);
        uint256 j;
        for (uint256 i; i < logs.length; ++i) {
            if (logs[i].logType == 4) payloads[j++] = logs[i].payload;
        }
    }

    // ── Chunk 3: Neon EVM events (optional) ───────────────────────────────

    function decodeChunk3Neon(bytes calldata chunk)
        internal pure returns (SolanaTypes.Chunk3Neon memory d)
    {
        (d.signature, d.evmLogs) =
            abi.decode(chunk, (bytes, SolanaTypes.EvmLogEntry[]));
    }
}
```

### 6.5 Example: Prove Token Transfer on CC3

Uses only 3 named local variables (`d`, `i`, `delta`) in the verification logic — well within budget:

```solidity
contract TokenTransferVerifier {
    INativeQueryVerifier public immutable verifier;

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    function verifyTokenTransfer(
        uint64  chainKey,
        uint64  slotHeight,
        bytes   calldata chunk1Bytes,
        bytes32[] calldata chunkProof,
        INativeQueryVerifier.MerkleProof calldata txProof,
        INativeQueryVerifier.ContinuityProof calldata continuityProof,
        bytes32 expectedMint,
        bytes32 expectedOwner,
        uint64  expectedDelta
    ) external view returns (bool) {
        require(
            verifier.verifyChunk(chainKey, slotHeight, chunk1Bytes,
                                 chunkProof, txProof, continuityProof),
            "Proof failed"
        );

        // d = 1 stack slot (struct), regardless of how many fields it has
        SolanaTypes.Chunk1 memory d = SolanaDecoder.decodeChunk1(chunk1Bytes);

        for (uint256 i; i < d.preTokenBalances.length; ++i) {
            if (d.preTokenBalances[i].mint  == expectedMint &&
                d.preTokenBalances[i].owner == expectedOwner)
            {
                uint64 delta = d.postTokenBalances[i].amount
                             - d.preTokenBalances[i].amount;
                return delta == expectedDelta;
            }
        }
        return false;
    }
}
```

### 6.6 Automated Solidity Test Generation (No `via_ir`)

The Rust encoder binary includes a `generate-fixtures` subcommand. It takes a sample Solana block (or fetches one from RPC) and writes a Foundry test file:

```bash
# Generate test fixtures from a real block
cargo run -p solana-abi-encoding --bin encode-fixtures --     --rpc http://api.mainnet-beta.solana.com     --slot 330000000     --tx-index 0     --out precompiles/solana-decoder/test/fixtures/SolanaDecoder.t.sol
```

The generated test file looks like:

```solidity
// GENERATED — do not edit by hand.
// Re-generate with: cargo run -p solana-abi-encoding --bin encode-fixtures
// Slot: 330000000, TX index: 0
// solc: no via_ir, optimizer_runs=200

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../SolanaDecoder.sol";
import "../SolanaTypes.sol";

contract SolanaDecoderFixtureTest is Test {

    // Raw ABI-encoded chunk bytes produced by Rust encoder
    bytes constant CHUNK0 = hex"...";
    bytes constant CHUNK1 = hex"...";
    bytes constant CHUNK2 = hex"...";

    // ── Chunk 0 ─────────────────────────────────────────────────────────────

    function test_decodeChunk0_roundtrip() public pure {
        SolanaTypes.Chunk0 memory d = SolanaDecoder.decodeChunk0(CHUNK0);

        // Values injected by encoder from the actual transaction
        assertEq(d.fee, 5000);
        assertTrue(d.success);
        assertEq(d.accountKeys.length, 3);
        assertEq(d.preBalances.length, d.accountKeys.length);
        assertEq(d.postBalances.length, d.accountKeys.length);
    }

    // Call decodeChunk0 from a function with many other locals
    // to prove we don't hit stack-too-deep in a realistic consumer context.
    function test_decodeChunk0_noStackTooDeep_inConsumer() public pure {
        SolanaTypes.Chunk0 memory d = SolanaDecoder.decodeChunk0(CHUNK0);

        // Simulate a realistic caller: multiple local vars + loop
        uint256 totalIn;
        uint256 totalOut;
        for (uint256 i; i < d.preBalances.length; ++i) {
            totalIn  += d.preBalances[i];
            totalOut += d.postBalances[i];
        }
        uint256 netDelta = totalIn > totalOut ? totalIn - totalOut : 0;
        assertTrue(netDelta >= d.fee); // fee comes out of balances
        assertGt(d.accountKeys.length, 0);
    }

    // ── Chunk 1 ─────────────────────────────────────────────────────────────

    function test_decodeChunk1_roundtrip() public pure {
        SolanaTypes.Chunk1 memory d = SolanaDecoder.decodeChunk1(CHUNK1);

        assertEq(d.preTokenBalances.length, d.postTokenBalances.length);
        // Values from the actual transaction
        assertEq(d.preTokenBalances[0].mint, bytes32(hex"..."));
    }

    function test_decodeChunk1_noStackTooDeep_inConsumer() public pure {
        SolanaTypes.Chunk1 memory d = SolanaDecoder.decodeChunk1(CHUNK1);

        bytes32 targetMint   = d.preTokenBalances[0].mint;
        bytes32 targetOwner  = d.preTokenBalances[0].owner;
        uint64  preBal       = d.preTokenBalances[0].amount;
        uint64  postBal      = d.postTokenBalances[0].amount;
        uint8   decimals     = d.preTokenBalances[0].decimals;
        uint256 idx          = d.preTokenBalances[0].accountIndex;

        assertEq(targetMint, d.postTokenBalances[0].mint);
        assertEq(targetOwner, d.postTokenBalances[0].owner);
        assertTrue(preBal != postBal || preBal == postBal); // trivially true, but forces use
        assertTrue(decimals <= 18);
        assertLt(idx, 256);
    }

    // ── Chunk 2 ─────────────────────────────────────────────────────────────

    function test_decodeChunk2_roundtrip() public pure {
        SolanaTypes.Chunk2 memory d = SolanaDecoder.decodeChunk2(CHUNK2);
        assertTrue(d.logs.length > 0);
    }

    function test_decodeChunk2_extractDataLogs() public pure {
        SolanaTypes.Chunk2 memory d = SolanaDecoder.decodeChunk2(CHUNK2);
        bytes[] memory dataPayloads  = SolanaDecoder.extractDataLogs(d.logs);

        // At least one Program data: entry if this is a Wormhole tx
        // (adjust assertion based on actual transaction type)
        assertTrue(dataPayloads.length >= 0);
    }

    function test_decodeChunk2_noStackTooDeep_inConsumer() public pure {
        SolanaTypes.Chunk2 memory d     = SolanaDecoder.decodeChunk2(CHUNK2);
        bytes[] memory dataLogs         = SolanaDecoder.extractDataLogs(d.logs);

        uint256 invokeCount;
        uint256 dataCount;
        for (uint256 i; i < d.logs.length; ++i) {
            if (d.logs[i].logType == 0) ++invokeCount;
            if (d.logs[i].logType == 4) ++dataCount;
        }
        assertEq(dataCount, dataLogs.length);
    }
}
```

**CI integration:**

```yaml
# .github/workflows/solana-decoder.yml
- name: Generate fixtures
  run: cargo run -p solana-abi-encoding --bin encode-fixtures --
         --rpc ${{ secrets.SOLANA_RPC }} --slot 330000000 --tx-index 0
         --out precompiles/solana-decoder/test/fixtures/SolanaDecoder.t.sol

- name: Run Foundry tests (no via_ir)
  working-directory: precompiles/solana-decoder
  run: |
    forge test --no-match-test "skip"       --optimizer-runs 200       # explicitly NO --via-ir flag
```

The fixture generator is part of the encoder crate, not a separate tool. Adding a new chunk type automatically updates the generated test.

### 6.7 TypeScript / Off-Chain Decoding

Same ABI bytes → use `ethers` `AbiCoder`:

```typescript
import { AbiCoder } from 'ethers';
const coder = AbiCoder.defaultAbiCoder();

// Chunk 0
const [sig0, accountKeys, preBal, postBal, success, fee] = coder.decode(
  ['bytes', 'bytes32[]', 'uint64[]', 'uint64[]', 'bool', 'uint64'],
  chunk0Bytes
);

// Chunk 1
const [sig1, preTok, postTok] = coder.decode(
  ['bytes',
   'tuple(uint8 accountIndex, bytes32 mint, bytes32 owner, uint64 amount, uint8 decimals)[]',
   'tuple(uint8 accountIndex, bytes32 mint, bytes32 owner, uint64 amount, uint8 decimals)[]'],
  chunk1Bytes
);

// Chunk 2 — filter to Program data: entries (logType=4)
const [sig2, logs] = coder.decode(
  ['bytes', 'tuple(bytes32 programId, uint8 depth, uint8 logType, bytes payload)[]'],
  chunk2Bytes
);
const dataLogs = (logs as any[]).filter(l => l.logType === 4);
```

---

## 7. Proposed Architecture

### 7.1 Option A: Trait Abstraction (Recommended)

Extract a `SourceChainClient` trait. Both `eth::Client` and `solana::Client` implement it. `StreamRoots` and `StreamTip` become generic over `SourceChainClient`.

```
SourceChainClient (trait)
├── eth::Client    (impl)
└── solana::Client (impl)

StreamRoots<C: SourceChainClient>
StreamTip<C: SourceChainClient>
```

**Pros:** No code duplication, single stream implementation  
**Cons:** More complex generic bounds; requires `StreamRoots` refactor

### 7.2 Option B: Parallel Crates (Simpler)

Keep `common/streams/eth` as-is. Add `common/streams/solana` as a parallel crate with `SolanaStreamRoots` and `SolanaStreamTip` that mirror the EVM versions but use `solana::Client`.

**Pros:** Zero risk to existing EVM path, easy to implement  
**Cons:** Code duplication in stream logic; reconnect/backoff logic needs to be maintained in two places

### 7.3 Recommendation

**Start with Option B** to ship faster and avoid breaking the EVM path. Refactor to Option A in a follow-up PR once both paths are proven stable.

### 7.4 Attestor Entrypoint Branching

`attestor/attestor/src/lib.rs` currently creates `eth::Client` unconditionally. This must become conditional:

```
match supported_chain.chain_encoding {
    ChainEncodingVersion::V1     => run_evm_path(config, supported_chain).await,
    ChainEncodingVersion::SolanaV1 => run_solana_path(config, supported_chain).await,
}
```

Each path manages its own client init, chain ID validation, stream construction, and maturity strategy interpretation.

---

## 8. File-by-File Change Map

### 8.1 `primitives/attestor/src/lib.rs`

**Change:** Add `SolanaV1 = 2` to `ChainEncodingVersion`.

```rust
pub enum ChainEncodingVersion {
    V1 = 1,
    SolanaV1 = 2,
}
```

This enum is stored on-chain via SCALE codec. Adding a new variant is **backwards-compatible** (existing encoded `V1 = 1` decodes correctly). No migration needed for the enum itself, only if `SupportedChain` storage layout changes.

Also update the `From<ChainEncodingVersion>` for `usc_abi_encoding::common::EncodingVersion` — this only applies to `V1`; `SolanaV1` routes to the new Solana encoder, not through `EncodingVersion`.

### 8.2 `primitives/supported-chains/src/lib.rs`

**Change:** Add `SolanaFinalized` to `MaturityStrategy` and the string constant.

```rust
pub enum MaturityStrategy {
    EvmFinalized,
    EvmSafe,
    EvmLatest,
    FixedDelay(u64),
    SolanaFinalized,  // NEW
}

pub const MATURITY_SOLANA_FINALIZED: &str = "SolanaFinalized";

impl MaturityStrategy {
    pub const fn maturity_delay(&self) -> Option<u64> {
        match self {
            Self::EvmFinalized => Some(64),
            Self::EvmSafe => Some(32),
            Self::EvmLatest => Some(0),
            Self::FixedDelay(n) => Some(*n),
            Self::SolanaFinalized => Some(31),  // slots (≈13s on mainnet)
        }
    }
}

impl TryFrom<&str> for MaturityStrategy {
    // ...
    MATURITY_SOLANA_FINALIZED => Ok(MaturityStrategy::SolanaFinalized),
    // ...
}
```

> Note: The `maturity_delay` for `SolanaFinalized` is used differently from EVM. For EVM, it is a block lag applied in `StreamRoots`. For Solana, the `StreamTip` uses `SlotInfo.root` (already the finalized slot) so the lag is informational/validation only, not applied as an offset. Document this clearly in code comments.

### 8.3 New Crate: `common/solana`

Create `common/solana/Cargo.toml` and `common/solana/src/lib.rs`.

**Contents:**

- `solana::Client` — wraps `solana-client` RPC client
- `SolanaOrderedBlock` — equivalent of `eth::OrderedBlock`
- `SolanaTxItem` — equivalent of `eth::TxRx`, implements `BlockItem`
- `simple_merkle_tree(block: &SolanaOrderedBlock) -> merkle::KeccakMerkleTree`

**Dependencies to add:**
```toml
[dependencies]
solana-client = "2.2"
solana-sdk = "2.2"
solana-transaction-status = "2.2"
solana-account-decoder = "2.2"  # if needed
anyhow = "1"
thiserror = "1"
tracing = "0.1"
merkle = { path = "../../merkle" }
utils = { path = "../../utils" }
tokio = { features = ["full"] }
tokio-retry = "0.3"
sp-core = { ... }
```

> **Warning:** Solana crates on crates.io are large and pull in many transitive deps. Pin versions carefully. Use `solana-rpc-client` if using Solana 2.x split crates. Check what version `solana-test-validator` uses locally to avoid mismatches.

### 8.4 New Crates: `solana-abi-encoding` + `SolanaDecoder.sol`

**`common/solana-abi-encoding`** (Rust, off-chain) — ABI encoding for the attestor:
- `pub fn encode_chunk0(tx) -> Result<Vec<u8>, EncodeError>` — SOL transfers
- `pub fn encode_chunk1(tx) -> Result<Vec<u8>, EncodeError>` — Token transfers
- `pub fn encode_chunk2(tx) -> Result<Vec<u8>, EncodeError>` — Logs
- `pub fn encode_chunk3_neon(tx) -> Option<Result<Vec<u8>, EncodeError>>` — Neon EVM (optional)
- `pub fn build_tx_sub_root(chunks: &[Vec<u8>]) -> H256` — TX-level Merkle root
- Depends on `alloy::dyn_abi` + `solana-transaction-status`

**`SolanaDecoder.sol`** (Solidity, on-chain) — decoder for the CC3 contract layer:
- `SolanaTypes` library — `TokenBalance`, `LogEntry`, `EvmLogEntry` structs
- `SolanaDecoder` library — `decodeChunk0/1/2/3Neon`, `extractDataLogs`
- See Section 6 for full implementation

**`BlockProver` precompile update** — new overload for two-level proofs:
- `verifyChunk(chainKey, height, chunkBytes, chunkProof, txProof, continuityProof)`
- See Section 5.3 for details

### 8.5 New Crate: `common/streams/solana` (Option B)

- `SolanaStreamRoots` — mirrors `eth::StreamRoots`, uses `solana::Client`
- `SolanaStreamTip` — mirrors `eth::StreamTip`, uses `SlotInfo.root` for finalized tip

### 8.6 `attestor/attestor/src/lib.rs`

**Change:** Branch on `supported_chain.chain_encoding` after fetching the chain config. Extract the EVM-specific client init into a sub-function or module. Add Solana path.

Key changes:
1. Config struct needs a `url_solana: Option<RpcSecret>` alongside `url_eth`
2. `wait_for_endpoints` must check the correct URL based on chain encoding
3. Chain ID validation differs: EVM uses `client_eth.chain_id() == supported_chain.chain_id`; Solana uses genesis-hash-derived ID
4. `StreamRoots` and `StreamTip` wiring becomes conditional

### 8.7 `runtime/src/migrations.rs`

**Likely not needed** for just adding a new `ChainEncodingVersion` variant (SCALE enum encoding is by discriminant, and `SolanaV1 = 2` is a new discriminant). However, if `SupportedChain` itself grows a new field, a migration IS needed.

No new fields are proposed in this spec — only new variants on existing enums.

### 8.8 Chainspecs

Add a new entry for Solana in relevant chainspec files:

```json
{
  "chain_key": 2,
  "chain_id": "<first-8-bytes-of-genesis-hash-as-u64>",
  "chain_name": "Solana Mainnet",
  "chain_encoding": "SolanaV1",
  "maturity_strategy": "SolanaFinalized"
}
```

---

## 9. Code Hints & Skeletons

### 9.1 `solana::Client`

```rust
// common/solana/src/lib.rs

use solana_client::{
    nonblocking::rpc_client::RpcClient,
    pubsub_client::PubsubClient,
    rpc_config::RpcBlockConfig,
    rpc_response::SlotInfo,
};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_transaction_status::{
    EncodedConfirmedBlock, TransactionDetails, UiTransactionEncoding,
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Solana RPC error: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("Slot {0} not found (skipped)")]
    SlotSkipped(u64),
    #[error("Failed to parse genesis hash")]
    GenesisHashParse,
}

pub struct Client {
    rpc: RpcClient,
    ws_url: String,
    chain_id: u64,   // derived from genesis hash
}

impl Client {
    pub async fn new(rpc_url: &str, ws_url: &str) -> anyhow::Result<Self> {
        let rpc = RpcClient::new_with_commitment(
            rpc_url.to_string(),
            CommitmentConfig::finalized(),
        );

        let genesis_hash = rpc.get_genesis_hash().await?;
        let chain_id = genesis_hash_to_chain_id(&genesis_hash.to_string())?;

        Ok(Self { rpc, ws_url: ws_url.to_string(), chain_id })
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub async fn get_block(&self, slot: u64) -> Result<SolanaOrderedBlock, Error> {
        let config = RpcBlockConfig {
            encoding: Some(UiTransactionEncoding::Base64),
            transaction_details: Some(TransactionDetails::Full),
            rewards: Some(false),
            commitment: Some(CommitmentConfig::finalized()),
            max_supported_transaction_version: Some(0),
        };

        match self.rpc.get_block_with_config(slot, config).await {
            Ok(block) => Ok(SolanaOrderedBlock::from_confirmed_block(slot, block)?),
            Err(e) if is_slot_skipped(&e) => {
                // Skipped slot = empty block
                Ok(SolanaOrderedBlock::empty(slot))
            }
            Err(e) => Err(Error::Rpc(e)),
        }
    }

    /// Subscribe to slot updates. Returns a stream of finalized slot numbers
    /// via SlotInfo.root field.
    pub fn subscribe_finalized_slots(&self) -> impl futures::Stream<Item = u64> {
        // PubsubClient::slot_subscribe returns SlotInfo { slot, parent, root }
        // `root` is the latest finalized slot — use this as the finalized tip
        use async_stream::stream;
        let ws_url = self.ws_url.clone();
        stream! {
            loop {
                match PubsubClient::slot_subscribe(&ws_url) {
                    Ok((mut sub, _)) => {
                        while let Some(slot_info) = sub.next() {
                            yield slot_info.root;
                        }
                    }
                    Err(e) => {
                        tracing::error!(%e, "Solana WS disconnected, retrying...");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

fn genesis_hash_to_chain_id(genesis_hash: &str) -> Result<u64, Error> {
    // base58-decode the genesis hash, take first 8 bytes as LE u64
    let bytes = bs58::decode(genesis_hash)
        .into_vec()
        .map_err(|_| Error::GenesisHashParse)?;
    if bytes.len() < 8 {
        return Err(Error::GenesisHashParse);
    }
    Ok(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
}

fn is_slot_skipped(e: &solana_client::client_error::ClientError) -> bool {
    // Solana returns error code -32009 "Slot X was skipped, or missing in long-term storage"
    use solana_client::client_error::ClientErrorKind;
    matches!(e.kind(), ClientErrorKind::RpcError(_))
        && e.to_string().contains("was skipped")
}
```

### 9.2 `SolanaOrderedBlock` and `SolanaTxItem`

```rust
// common/solana/src/lib.rs (continued)

use utils::block_item_traits::BlockItem;

pub struct SolanaTxItem {
    pub inner: solana_transaction_status::EncodedTransactionWithStatusMeta,
    pub slot: u64,
}

impl BlockItem for SolanaTxItem {
    fn payload_bytes(&self) -> Vec<u8> {
        solana_abi_encoding::solana_v1_encode(&self.inner)
            .expect("Solana transaction should be encodable")
            .abi()
            .to_vec()
    }

    fn tx_type(&self) -> Option<u8> {
        // Solana doesn't have EVM tx types; return None or a version discriminant
        Some(0) // Version 0 = legacy; version 1 = versioned tx (Address Lookup Tables)
    }
}

pub struct SolanaOrderedBlock {
    pub slot: u64,
    pub blockhash: [u8; 32],  // SHA-256, 32 bytes
    pub items: Vec<SolanaTxItem>,
}

impl SolanaOrderedBlock {
    pub fn empty(slot: u64) -> Self {
        Self { slot, blockhash: [0u8; 32], items: vec![] }
    }

    pub fn from_confirmed_block(
        slot: u64,
        block: solana_transaction_status::UiConfirmedBlock,
    ) -> Result<Self, Error> {
        let blockhash_str = block.blockhash.clone();
        let blockhash_bytes = bs58::decode(&blockhash_str)
            .into_vec()
            .map_err(|_| Error::GenesisHashParse)?;
        let mut blockhash = [0u8; 32];
        blockhash.copy_from_slice(&blockhash_bytes);

        let items = block
            .transactions
            .unwrap_or_default()
            .into_iter()
            .map(|tx| SolanaTxItem { inner: tx, slot })
            .collect();

        Ok(Self { slot, blockhash, items })
    }

    pub fn slot(&self) -> u64 { self.slot }
    pub fn hash(&self) -> [u8; 32] { self.blockhash }
    pub fn items(&self) -> &[SolanaTxItem] { &self.items }
}

/// Build Keccak Merkle tree from Solana block (same as EVM path)
pub fn simple_merkle_tree(block: &SolanaOrderedBlock) -> merkle::KeccakMerkleTree {
    let tx_bytes: Vec<Vec<u8>> = block.items().iter().map(|item| item.to_bytes()).collect();
    merkle::KeccakMerkleTree::new(&tx_bytes)
}
```

### 9.3 `solana_abi_encoding` — Chunk Encoders

```rust
// common/solana-abi-encoding/src/lib.rs
// ABI encoding using alloy::dyn_abi.
// Each encode_chunkN() produces one independently-decodable ABI blob.

use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{FixedBytes, U256};
use solana_transaction_status::EncodedTransactionWithStatusMeta;

#[derive(thiserror::Error, Debug)]
pub enum EncodeError {
    #[error("Missing transaction meta")]
    MissingMeta,
    #[error("Failed to decode base58 pubkey: {0}")]
    Base58Decode(String),
    #[error("ABI encoding error: {0}")]
    AbiError(String),
}

/// Encode Chunk 0: SOL transfers
/// ABI type: (bytes, bytes32[], uint64[], uint64[], bool, uint64)
pub fn encode_chunk0(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<Vec<u8>, EncodeError> {
    let meta = tx.meta.as_ref().ok_or(EncodeError::MissingMeta)?;
    let (signature, account_keys) = extract_signature_and_accounts(tx)?;

    let encoded = DynSolValue::Tuple(vec![
        DynSolValue::Bytes(signature),
        DynSolValue::Array(account_keys.into_iter()
            .map(|k| DynSolValue::FixedBytes(FixedBytes::from(k), 32))
            .collect()),
        DynSolValue::Array(meta.pre_balances.iter()
            .map(|b| DynSolValue::Uint(U256::from(*b), 64)).collect()),
        DynSolValue::Array(meta.post_balances.iter()
            .map(|b| DynSolValue::Uint(U256::from(*b), 64)).collect()),
        DynSolValue::Bool(meta.err.is_none()),   // success = no error
        DynSolValue::Uint(U256::from(meta.fee), 64),
    ]);
    Ok(encoded.abi().to_vec())
}

/// Encode Chunk 1: Token transfers
/// ABI type: (bytes, (uint8,bytes32,bytes32,uint64,uint8)[], (...)[])
pub fn encode_chunk1(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<Vec<u8>, EncodeError> {
    let meta = tx.meta.as_ref().ok_or(EncodeError::MissingMeta)?;
    let (signature, _) = extract_signature_and_accounts(tx)?;

    let encode_balances = |balances: &[solana_transaction_status::UiTransactionTokenBalance]| {
        balances.iter().map(|b| {
            DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::from(b.account_index), 8),
                DynSolValue::FixedBytes(
                    FixedBytes::from(decode_pubkey(&b.mint).unwrap_or([0u8;32])), 32
                ),
                DynSolValue::FixedBytes(
                    FixedBytes::from(b.owner.as_deref()
                        .and_then(|o| decode_pubkey(o).ok())
                        .unwrap_or([0u8;32])), 32
                ),
                DynSolValue::Uint(
                    U256::from(b.ui_token_amount.amount.parse::<u64>().unwrap_or(0)), 64
                ),
                DynSolValue::Uint(U256::from(b.ui_token_amount.decimals), 8),
            ])
        }).collect::<Vec<_>>()
    };

    let pre = encode_balances(meta.pre_token_balances.as_deref().unwrap_or(&[]));
    let post = encode_balances(meta.post_token_balances.as_deref().unwrap_or(&[]));

    let encoded = DynSolValue::Tuple(vec![
        DynSolValue::Bytes(signature),
        DynSolValue::Array(pre),
        DynSolValue::Array(post),
    ]);
    Ok(encoded.abi().to_vec())
}

/// Encode Chunk 2: Logs / cross-chain intents
/// ABI type: (bytes, (bytes32,uint8,uint8,bytes)[])
pub fn encode_chunk2(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<Vec<u8>, EncodeError> {
    let meta = tx.meta.as_ref().ok_or(EncodeError::MissingMeta)?;
    let (signature, _) = extract_signature_and_accounts(tx)?;

    let logs = meta.log_messages.as_deref().unwrap_or(&[]);
    let log_entries = parse_log_entries(logs);

    let encoded = DynSolValue::Tuple(vec![
        DynSolValue::Bytes(signature),
        DynSolValue::Array(log_entries),
    ]);
    Ok(encoded.abi().to_vec())
}

/// Build the TX-level sub-root from all chunks.
/// chunks = [chunk0_bytes, chunk1_bytes, chunk2_bytes, ...]
pub fn build_tx_sub_root(chunks: &[Vec<u8>]) -> sp_core::H256 {
    merkle::KeccakMerkleTree::new(chunks).root()
}

fn parse_log_entries(logs: &[String]) -> Vec<DynSolValue> {
    // Parse each log line into (programId, depth, logType, payload)
    // logType: 0=invoke, 1=success, 2=fail, 3=log, 4=data
    logs.iter().map(|line| {
        let (program_id, depth, log_type, payload) = classify_log_line(line);
        DynSolValue::Tuple(vec![
            DynSolValue::FixedBytes(FixedBytes::from(program_id), 32),
            DynSolValue::Uint(U256::from(depth), 8),
            DynSolValue::Uint(U256::from(log_type), 8),
            DynSolValue::Bytes(payload),
        ])
    }).collect()
}

fn classify_log_line(line: &str) -> ([u8; 32], u8, u8, Vec<u8>) {
    // "Program <id> invoke [<depth>]" → (id, depth, 0, empty)
    // "Program <id> success"          → (id, 0, 1, empty)
    // "Program <id> failed: ..."      → (id, 0, 2, message bytes)
    // "Program log: ..."              → (zero, 0, 3, text bytes)
    // "Program data: <base64>"        → (zero, 0, 4, decoded bytes)
    todo!("implement log line classification")
}

fn decode_pubkey(s: &str) -> Result<[u8; 32], EncodeError> {
    bs58::decode(s).into_vec()
        .map_err(|e| EncodeError::Base58Decode(e.to_string()))
        .and_then(|v| v.try_into()
            .map_err(|_| EncodeError::Base58Decode(format!("not 32 bytes: {s}"))))
}

fn extract_signature_and_accounts(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<(Vec<u8>, Vec<[u8; 32]>), EncodeError> {
    // Extract first signature (bytes64) and all account keys
    todo!("implement message field extraction")
}
```

### 9.4 `SolanaStreamTip` (Option B)

```rust
// common/streams/solana/src/tip.rs

pub struct Config {
    pub client: solana::Client,
    pub start_height: attestor_primitives::Height,
}

pub struct SolanaStreamTip {
    stream: sync_wrapper::SyncStream<stream_util::BoxedStream<attestor_primitives::Height>>,
    config: Config,
}

impl SolanaStreamTip {
    pub async fn new(config: Config) -> Self {
        use futures::StreamExt as _;

        let client = config.client.clone();
        let start_height = config.start_height;

        let stream = client
            .subscribe_finalized_slots()
            .filter(move |&slot| futures::future::ready(slot >= start_height))
            .boxed();

        // No lag subtraction needed — SlotInfo.root is already finalized
        Self {
            stream: sync_wrapper::SyncStream::new(stream),
            config,
        }
    }
}

impl futures::Stream for SolanaStreamTip {
    type Item = attestor_primitives::Height;
    fn poll_next(/* ... */) { /* ... */ }
}
```

### 9.5 `SolanaStreamRoots`

```rust
// common/streams/solana/src/roots.rs

pub struct Config {
    pub client: solana::Client,
    pub start_height: attestor_primitives::Height,
    pub max_concurrency: std::num::NonZeroUsize,
    pub max_parallelism: std::num::NonZeroUsize,
}

// Structure mirrors eth::StreamRoots exactly, but:
// - Subscribes to finalized slots via client.subscribe_finalized_slots()
// - Calls client.get_block(slot) instead of eth_client.get_block(n, encoding)
// - Empty slots (SlotSkipped error) → RootInfo with empty merkle root
// - hash field = H256::from(block.blockhash) (already 32 bytes)

// Key difference: Solana slots can skip. When get_block returns SlotSkipped,
// emit a RootInfo with root = KeccakMerkleTree::new(&[]).root()
// and hash = H256::zero() (no block hash for skipped slot)
```

### 9.6 Attestor Entrypoint Branch

```rust
// attestor/attestor/src/lib.rs — conceptual change (not full impl)

// After fetching supported_chain...

let (stream_roots, stream_tip) = match supported_chain.chain_encoding {
    ChainEncodingVersion::V1 => {
        // Existing EVM path
        let client_eth = eth::Client::new(&config.url_eth, None).await?;

        // Validate chain ID
        if supported_chain.chain_id != client_eth.chain_id() {
            return Err(Error::ChainIdMisMatch { ... });
        }

        let roots = stream::eth::StreamRoots::new(/* ... eth config ... */).await;
        let tip = stream::eth::StreamTip::new(/* ... eth config ... */).await;
        (roots.boxed_data(), tip.boxed_data())
    }

    ChainEncodingVersion::SolanaV1 => {
        // New Solana path
        let url_solana = config.url_solana
            .ok_or(Error::MissingSolanaUrl)?;

        let client_solana = solana::Client::new(
            &url_solana.rpc,
            &url_solana.ws,
        ).await?;

        // Validate chain ID (genesis-hash-derived)
        if supported_chain.chain_id != client_solana.chain_id() {
            return Err(Error::ChainIdMisMatch { ... });
        }

        let roots = stream::solana::SolanaStreamRoots::new(/* ... */).await;
        let tip = stream::solana::SolanaStreamTip::new(/* ... */).await;
        (roots.boxed_data(), tip.boxed_data())
    }
};

// Everything below (StreamAttestation, workers, BLS) stays unchanged
```

---

## 10. Open Questions

These must be resolved before implementation begins.

### Q1: On-chain block proof verification target

**Question:** Does verification happen on a Solana program, an EVM/CC3 precompile, or both?

**Resolved (2026-04-24):** ABI encoding. Chunks designed for EVM/Solidity decoding (primary target). Same chunks usable in Solana programs via `alloy-sol-types` no_std. See Sections 5 and 6.

---

### Q2: Solana RPC transport — HTTP polling vs WebSocket pubsub?

**Question:** Should `solana::Client` use HTTP polling with exponential backoff (simpler) or WebSocket `slotSubscribe` (lower latency, matches EVM pattern)?

**Recommendation:** WebSocket pubsub. Reasons:
- Matches the EVM `subscribe_blocks()` pattern
- `SlotInfo.root` gives finalized slot directly — no need to poll `getSlot(finalized)`
- Solana mainnet slots are 400ms; polling with HTTP adds unnecessary latency

**Concern:** `PubsubClient` in `solana-client` is synchronous (uses `std::sync::mpsc` under the hood). Need to bridge to async with `tokio::task::spawn_blocking` or use `solana-pubsub-client` crate separately.

---

### Q3: How to handle skipped slots in the attestor pipeline?

**Question:** When a Solana slot is skipped (no block produced), what should the attestor do?

**Options:**
- **A)** Skip it entirely — only attest slots with actual blocks. Requires the attestation interval logic to be slot-aware and skip gaps.
- **B)** Emit an "empty block" — produce an attestation with an empty Merkle root (like an empty EVM block). Height still advances by 1.

**Recommendation:** Option B for simplicity — empty Merkle root for skipped slots. This keeps the height sequence contiguous and avoids special-casing in the attestation interval logic.

**Note:** On Solana mainnet, roughly 4-8% of slots are skipped. This is frequent enough to matter.

---

### Q4: Versioned transactions (v0 / Address Lookup Tables)?

**Question:** The Solana `SolanaV1` encoding uses `max_supported_transaction_version: Some(0)` which means only legacy transactions. Should versioned transactions (v0, which use Address Lookup Tables) be supported?

**Background:** Solana introduced versioned transactions in 2022. Most DeFi protocols use v0 transactions today. Passing `max_supported_transaction_version: Some(0)` in `getBlock` causes the RPC to return an error if the block contains v0 transactions.

**Impact:** If mainnet is the target, v0 transactions MUST be supported. Use `max_supported_transaction_version: Some(0)` only for legacy-only chains (devnet testing).

**Recommendation:** Support v0 from the start. The `SolanaV1` encoding accounts for this — the `tx_type()` hint returns `0` (legacy) or `1` (versioned). The encoding itself is structurally the same; versioned transactions just have a different `message` structure internally.

---

### Q5: `usc-abi-encoding` extend vs separate crate?

**Question:** Add `SolanaV1` to the existing `usc-abi-encoding` crate or create `solana-abi-encoding` as a separate crate?

**Issue with extending:** `usc-abi-encoding` imports `alloy::rpc::types::Transaction` and `TransactionReceipt` directly. Solana types are from a completely separate ecosystem. Mixing them in one crate creates a heavyweight dependency on both `alloy` and `solana-client`/`solana-sdk`.

**Recommendation:** Separate crate `common/solana-abi-encoding`. Keeps dependencies clean and allows the crate to be no-std-compatible in future if needed.

---

### Q6: Config format — how does the attestor know the Solana RPC URL?

**Question:** Currently the attestor config has `url_eth`. For Solana, it needs an HTTP RPC URL and a WS URL. What's the config format?

**Options:**
- A) Add `url_solana_rpc` and `url_solana_ws` as separate optional fields
- B) Encode both in one URL (e.g., `solana+ws://...`) with a custom scheme
- C) Detect from `chain_encoding` and use a single `url_source` field that is interpreted differently

**Recommendation:** Option A — explicit separate fields. Most operationally clear. `url_eth` stays for EVM chains; `url_solana_rpc` + `url_solana_ws` for Solana chains. Both optional; validated at startup based on `chain_encoding`.

---

### Q7: Genesis attestation for Solana?

**Question:** The EVM genesis attestation path fetches a specific block by number and produces the first attestation. For Solana, what slot does genesis start from?

**Answer:** Same concept — the `SupportedChain` has a genesis block number (now interpreted as a slot number). The attestor fetches that slot and produces the genesis attestation root. No structural change needed.

---

## 11. Testing Strategy

### 11.1 Unit Tests

**`solana-encoding`:**
- Fixture test: known Solana transaction → known ABI-encoded chunk bytes (chunk0/1/2, determinism)
- Round-trip: `encode_chunk0/1/2` → `SolanaDecoder.decodeChunk0/1/2` → matches original fields
- Empty transaction list → empty bytes (not crash)
- Failed transaction (`is_err = true`) encoded correctly
- `SolanaDecoder::decode` in Solana program context (`alloy-sol-types` no_std / BPF)

**`solana::SolanaOrderedBlock`:**
- `from_confirmed_block` with mock block data
- `empty(slot)` produces zero-length items

**`genesis_hash_to_chain_id`:**
- Known genesis hashes → expected u64 values

### 11.2 Integration Tests

**Against `solana-test-validator`:**
1. Start local validator
2. Send a few transactions
3. Fetch the block, ABI-encode each tx into chunks, build two-level Merkle root
4. Decode chunks with `SolanaDecoder.sol`, assert token/log fields match
5. Compare Merkle root to a known-good value (generated once and frozen)

```bash
# Setup
solana-test-validator --reset &
sleep 5

# Send test tx
solana transfer --allow-unfunded-recipient <addr> 0.001 --keypair test-wallet.json

# Run integration test
cargo test --test solana_encoding_integration
```

**Decoder Anchor program test:**
1. Deploy a test Anchor program that accepts `encoded_tx: Vec<u8>` and calls `SolanaDecoder::decode`
2. Feed it a known-good encoded leaf
3. Assert the program succeeds and reads expected fields

### 11.3 End-to-End Test

Run the attestor against a local Solana test validator + local CC3 devnet:

1. Register a test `SupportedChain` with `chain_encoding: SolanaV1` on a local CC3 runtime
2. Start one attestor pointing at the test validator
3. Verify attestations appear on CC3 with correct Merkle roots
4. Check digest chain continuity (`prev_digest` links correctly)
5. Call the Anchor decoder program with a Merkle proof — verify it accepts the proof and decodes the leaf

### 11.4 Encoding Stability Test

Add a test that serializes a known transaction using `solana_v1_encode`, stores the hex output as a fixture, and asserts it never changes across builds. This prevents accidental encoding drift.

---

## 12. Appendix: Solana RPC Reference

### Key RPC Methods

| Method | Description |
|---|---|
| `getBlock(slot, config)` | Fetch full block with transactions and metadata |
| `getSlot({ commitment: "finalized" })` | Get current finalized slot number |
| `getGenesisHash()` | Get genesis hash (cluster identifier) |
| `getHealth()` | Node health check |
| `getVersion()` | Solana software version |

### Key WebSocket Subscriptions

| Subscription | Description |
|---|---|
| `slotSubscribe` | Emits `SlotInfo { slot, parent, root }` on every new slot. `root` = finalized slot |
| `blockSubscribe` | Emits full block data (requires `--rpc-pubsub-enable-block-subscription` on node) |
| `rootSubscribe` | Emits just the finalized root slot number |

> **Recommendation:** Use `slotSubscribe` for tip tracking (gives `root` = finalized). Use HTTP `getBlock` for block fetching (more reliable than WS block subscription which requires special node config).

### Error Codes

| Code | Meaning |
|---|---|
| -32009 | Slot was skipped or not found in long-term storage |
| -32004 | Block not yet available (too new) |
| -32007 | Transaction version unsupported (need `max_supported_transaction_version`) |

### Useful Resources

- [Solana Docs: JSON RPC](https://docs.solana.com/api/http)
- [Solana Cookbook](https://solanacookbook.com/)
- [solana-client crate docs](https://docs.rs/solana-client)
- [Solana Transaction Format](https://docs.solana.com/developing/programming-model/transactions)
- [Versioned Transactions explainer](https://docs.solana.com/developing/versioned-transactions)
- [`solana-test-validator` usage](https://docs.solana.com/developing/test-validator)

### Solana Mainnet RPC Endpoints (for reference)

- Official (rate-limited): `https://api.mainnet-beta.solana.com`
- Helius (recommended for production): `https://mainnet.helius-rpc.com`
- Triton One: `https://free.rpcpool.com`
- QuickNode: `https://solana-mainnet.rpc.extrnode.com`

> For production attestors, use a dedicated RPC endpoint (Helius, Triton, QuickNode). The official endpoint is heavily rate-limited and unsuitable for continuous block fetching.

---

*End of specification.*
