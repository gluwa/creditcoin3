# Solana Attestor Support — Technical Specification

**Status:** Draft  
**Author:** Protocol Engineering  
**Target Repo:** `gluwa/creditcoin3`  
**Last Updated:** 2026-04-24 (rev 3 — reverted to ABI encoding; SolanaDecoder uses alloy-sol-types no_std)

---

## Table of Contents

1. [Overview](#1-overview)
2. [Background: How Solana Works](#2-background-how-solana-works)
3. [Current EVM Pipeline (Reference)](#3-current-evm-pipeline-reference)
4. [What Changes for Solana](#4-what-changes-for-solana)
5. [ABI Encoding Design (SolanaV1)](#5-abi-encoding-design-solanav1)
6. [SolanaDecoder — ABI Decoding in Solana Programs](#6-solanadecoder--abi-decoding-in-solana-programs)
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

The `SolanaV1` encoding uses **Ethereum ABI encoding** — the same format as the existing EVM `V1` path. This ensures:

- Consistency across all source chains in the attestation pipeline
- Compatibility with the existing `QueryBuilder` offset-based field extraction infrastructure
- Future ability to verify Solana attestations from EVM contracts
- A single encoding/decoding codebase (`alloy-sol-types` in Rust, `ethers`/`viem` in TypeScript)
- Decoding in Solana programs via `alloy-sol-types` `no_std` mode (see Section 6)

> **Why not Borsh?** Borsh is Solana-native but cannot be decoded on EVM without a custom decoder. ABI can be decoded on both EVM (trivially) and in Solana programs (`alloy-sol-types` supports BPF target with `default-features = false`). One encoding for both chains.

The encoding must be:

1. **Deterministic** — same transaction always produces the same bytes
2. **ABI-compatible** — decodable by `alloy-sol-types` in Rust and `ethers`/`viem` in TypeScript
3. **Complete** — captures enough data for a meaningful attestation
4. **Stable** — adding new Solana transaction versions must not break existing attestations

### 5.1 ABI Struct Layout

The top-level shape mirrors the EVM `V1` encoding:

```
DynSolValue::Tuple([
    DynSolValue::Uint(version, 8),   // always 0 for SolanaV1
    DynSolValue::Array([
        Bytes(header_chunk),          // accounts + recent_blockhash
        Bytes(instructions_chunk),    // all instructions
        Bytes(meta_chunk),            // execution outcome
    ])
])
```

**`header_chunk`** — `abi.encode(bytes32 fee_payer, bytes32 recent_blockhash, bytes32[] account_keys)`:
```
abi.encode(
    bytes32 fee_payer,         // account_keys[0], base58-decoded pubkey
    bytes32 recent_blockhash,  // 32-byte hash, base58-decoded
    bytes32[] account_keys,    // all pubkeys, each base58-decoded to 32 bytes
)
```

**`instructions_chunk`** — `abi.encode(Instruction[])`:
```
abi.encode(
    tuple(uint8 program_id_index, uint8[] account_indices, bytes data)[]
)
```

**`meta_chunk`** — `abi.encode(bool, uint64, uint64[], uint64[], bytes[])`:
```
abi.encode(
    bool     is_err,           // true if tx failed
    uint64   fee,              // lamports
    uint64[] pre_balances,     // parallel to account_keys
    uint64[] post_balances,    // parallel to account_keys
    bytes[]  log_messages,     // UTF-8 log lines as bytes
)
```

### 5.2 Serialization (Attestor Side)

```rust
use alloy::dyn_abi::DynSolValue;

impl BlockItem for SolanaTxItem {
    fn payload_bytes(&self) -> Vec<u8> {
        solana_abi_encoding::solana_v1_encode(&self.inner)
            .expect("Solana transaction should be encodable")
            .abi()
            .to_vec()
    }
}
```

Resulting `Vec<u8>` is fed into `merkle::KeccakMerkleTree` — identical to the EVM path.

### 5.3 Field Exclusions and Rationale

- **No signatures** — excluded. Signatures don't affect execution outcome and would make the leaf hash non-deterministic across signature variants of the same message
- **No inner instructions** — excluded in v0 for simplicity; can be added in `SolanaV2` via the `version` field
- **No address lookup tables** — excluded in v0; relevant only for versioned transactions (see Open Question 4)
- **`meta` always included** — unlike EVM where the receipt is a separate RPC fetch, Solana `meta` is always returned alongside the transaction. Including it means the Merkle root captures execution outcome, not just intent
- **Log messages as `bytes[]`** — UTF-8 strings stored as `bytes` to avoid encoding ambiguity

### 5.4 Skipped Slots

If a slot was skipped (no block produced), emit an **empty Merkle tree** — same behavior as an empty EVM block:

```rust
merkle::KeccakMerkleTree::new(&[]).root() // → canonical empty root
```

### 5.5 Version Future-Proofing

The `uint8` version field at position 0 allows introducing `SolanaV2` (e.g., with inner instructions or address lookup table support) without changing the outer shape. Decoders branch on this byte before decoding the chunk array.

---

## 6. SolanaDecoder — ABI Decoding in Solana Programs

This section is the Solana equivalent of the EVM `QueryBuilder` / `EvmDecoder` pattern in `@gluwa/usc-sdk`. The encoding is ABI (same as EVM `V1`). The decoder runs inside a Solana program using `alloy-sol-types` in `no_std` mode.

### 6.1 EVM Decoder Recap (Reference)

The EVM path works like this:

```
ABI-encoded bytes
    │
    ▼  QueryBuilder (off-chain, TypeScript)
    │  Computes field byte offsets within ABI encoding
    │  Returns (offset, size) pairs → sent as calldata to Solidity
    ▼
Solidity on-chain
    assembly { calldataload(offset) }  → extracts field at known byte offset
```

ABI encoding uses fixed 32-byte slots so field byte offsets can be pre-computed off-chain. The Solidity contract does a cheap `calldataload(offset)`. `QueryBuilder` builds these offset descriptors.

### 6.2 Solana Decoder Pattern (ABI)

Same ABI bytes, different runtime. Solana programs are Rust `no_std` BPF binaries. `alloy-sol-types` (the `sol!` macro layer, no transport/provider) supports `no_std` since v0.7 and compiles for the BPF target with `default-features = false`.

**Pattern:**

```
ABI-encoded leaf bytes (from Merkle proof)
    │
    ▼  SolanaDecoder::decode() — in Solana program
    │  alloy_sol_types::SolType::abi_decode()
    ▼
Decoded Rust struct (zero-copy where possible)
    │  .fee_payer, .instructions[0].data, .is_err, etc.
    ▼
Field access — same struct, same ABI bytes as EVM
```

This means the same encoded `bytes` accepted by an EVM contract via `BlockProver` can also be decoded by a Solana program — no separate encoding per chain.

### 6.3 `alloy-sol-types` in a Solana Program

`alloy-sol-types` does **not** depend on `std` when `default-features = false`. It compiles to BPF cleanly. This is the key enabler.

```toml
# In your Solana program's Cargo.toml
[dependencies]
alloy-sol-types = { version = "0.8", default-features = false }
```

### 6.4 Solana Program: `SolanaDecoder`

Create a crate `solana-decoder` (usable both inside Solana programs and off-chain):

```rust
// solana-decoder/src/lib.rs
#![cfg_attr(target_arch = "bpf", no_std)]

use alloy_sol_types::{sol, SolType};

// Mirror the ABI types from SolanaV1 encoding
sol! {
    /// Header chunk: fee_payer, recent_blockhash, account_keys[]
    struct SolanaHeader {
        bytes32 fee_payer;
        bytes32 recent_blockhash;
        bytes32[] account_keys;
    }

    /// Single instruction
    struct SolanaIx {
        uint8 program_id_index;
        uint8[] account_indices;
        bytes data;
    }

    /// Meta / execution outcome chunk
    struct SolanaMeta {
        bool is_err;
        uint64 fee;
        uint64[] pre_balances;
        uint64[] post_balances;
        bytes[] log_messages;
    }

    /// Top-level leaf (version + array of encoded chunks)
    struct SolanaTxLeaf {
        uint8 version;
        bytes[] chunks; // [header_chunk, instructions_chunk, meta_chunk]
    }
}

pub struct SolanaDecoder;

impl SolanaDecoder {
    /// Decode the top-level ABI leaf and all sub-chunks.
    /// Returns (header, instructions, meta) or an error.
    pub fn decode(
        leaf_bytes: &[u8],
    ) -> Result<(SolanaHeader, Vec<SolanaIx>, SolanaMeta), alloy_sol_types::Error> {
        // 1. Decode the top-level tuple: (uint8 version, bytes[] chunks)
        let leaf = SolanaTxLeaf::abi_decode(leaf_bytes, true)?;

        let header_bytes = &leaf.chunks[0];
        let instructions_bytes = &leaf.chunks[1];
        let meta_bytes = &leaf.chunks[2];

        // 2. Decode each chunk independently
        let header = SolanaHeader::abi_decode(header_bytes, true)?;
        let instructions = <alloy_sol_types::sol_type::Array<SolanaIx>>::abi_decode(
            instructions_bytes, true
        )?;
        let meta = SolanaMeta::abi_decode(meta_bytes, true)?;

        Ok((header, instructions, meta))
    }

    /// Convenience: check if the attested transaction succeeded.
    pub fn succeeded(leaf_bytes: &[u8]) -> Result<bool, alloy_sol_types::Error> {
        let (_, _, meta) = Self::decode(leaf_bytes)?;
        Ok(!meta.is_err)
    }

    /// Convenience: get fee payer pubkey as [u8; 32].
    pub fn fee_payer(leaf_bytes: &[u8]) -> Result<[u8; 32], alloy_sol_types::Error> {
        let (header, _, _) = Self::decode(leaf_bytes)?;
        Ok(header.fee_payer.into())
    }

    /// Convenience: get raw instruction data at index.
    pub fn instruction_data(
        leaf_bytes: &[u8],
        ix_index: usize,
    ) -> Result<alloy_sol_types::private::Bytes, alloy_sol_types::Error> {
        let (_, instructions, _) = Self::decode(leaf_bytes)?;
        instructions
            .into_iter()
            .nth(ix_index)
            .map(|ix| ix.data)
            .ok_or(alloy_sol_types::Error::custom("instruction index out of bounds"))
    }
}
```

### 6.5 Usage in an Anchor Program

```rust
use solana_decoder::SolanaDecoder;

#[program]
pub mod my_program {
    use super::*;

    /// Verify an attested Solana transaction and act on its contents.
    pub fn process_attested_transfer(
        ctx: Context<ProcessAttestedTransfer>,
        encoded_tx: Vec<u8>,   // ABI-encoded SolanaV1 leaf bytes
        // merkle_proof, continuity_proof passed separately
    ) -> Result<()> {
        // Step 1: Verify Merkle + continuity proof
        // (Call CC3 attestation verifier program / CPI)
        verify_attestation_proof(&encoded_tx, &ctx.accounts)?;

        // Step 2: Decode the ABI leaf
        let (header, instructions, meta) = SolanaDecoder::decode(&encoded_tx)
            .map_err(|_| MyError::InvalidTxEncoding)?;

        // Step 3: Check transaction succeeded
        require!(!meta.is_err, MyError::TransactionFailed);

        // Step 4: Check fee payer matches expected sender
        let expected: [u8; 32] = ctx.accounts.sender.key().to_bytes();
        let fee_payer: [u8; 32] = header.fee_payer.into();
        require!(fee_payer == expected, MyError::WrongSender);

        // Step 5: Decode instruction data (application-specific)
        // SPL Token Transfer: ix.data[0] = 3 (discriminator), [1..9] = amount (LE u64)
        let ix = instructions.first().ok_or(MyError::MissingInstruction)?;
        let amount = u64::from_le_bytes(
            ix.data[1..9].try_into().map_err(|_| MyError::InvalidIxData)?
        );

        // Step 6: Act on verified, attested data
        transfer_tokens(ctx, amount)?;
        Ok(())
    }
}
```

### 6.6 TypeScript / Client-Side Decoding

Same ABI bytes → use `ethers`/`viem` `AbiCoder`, exactly as for EVM. No new tooling needed.

```typescript
import { AbiCoder } from 'ethers';

const coder = AbiCoder.defaultAbiCoder();

// Top-level: (uint8 version, bytes[] chunks)
const [version, chunks] = coder.decode(['uint8', 'bytes[]'], encodedLeaf);

// Header chunk
const [feePayer, recentBlockhash, accountKeys] = coder.decode(
  ['bytes32', 'bytes32', 'bytes32[]'],
  chunks[0]
);

// Instructions chunk — array of tuples
const instructions = coder.decode(
  ['tuple(uint8 program_id_index, uint8[] account_indices, bytes data)[]'],
  chunks[1]
)[0];

// Meta chunk
const [isErr, fee, preBalances, postBalances, logMessages] = coder.decode(
  ['bool', 'uint64', 'uint64[]', 'uint64[]', 'bytes[]'],
  chunks[2]
);

console.log('Fee payer:', Buffer.from(feePayer.slice(2), 'hex').toString('hex'));
console.log('Succeeded:', !isErr);
console.log('Fee (lamports):', fee.toString());
```

### 6.7 Comparison: EVM vs Solana Decoder

| Aspect | EVM (QueryBuilder + Solidity) | Solana (SolanaDecoder + alloy-sol-types) |
|---|---|---|
| Encoding | ABI | ABI (same bytes) |
| Off-chain offset tool | `QueryBuilder` (TypeScript) | Same `QueryBuilder` works |
| On-chain decode | `assembly { calldataload(offset) }` | `alloy-sol-types::SolType::abi_decode()` |
| Compute cost | Very cheap (single load) | Low-moderate (full ABI decode) |
| `no_std` support | N/A (EVM) | `alloy-sol-types` with `default-features = false` |
| TypeScript decode | `ethers AbiCoder` | Same `ethers AbiCoder` |
| Same bytes on EVM + Solana | N/A | ✅ Yes — one encoding for both chains |

**Key advantage of ABI over Borsh here:** The exact same encoded `leaf_bytes` can be verified by an EVM contract (`BlockProver` precompile) AND decoded by a Solana program. No dual encoding, no format divergence.

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

### 8.4 New Crates: `solana-abi-encoding` + `solana-decoder`

**`common/solana-abi-encoding`** — ABI encoding for the attestor side:
- `pub fn solana_v1_encode(tx: &EncodedTransactionWithStatusMeta) -> Result<DynSolValue, EncodeError>`
- Internal helpers: `encode_header`, `encode_instructions`, `encode_meta`
- Depends on `alloy::dyn_abi` + `solana-transaction-status`

**`solana-decoder`** — ABI decoding for Solana programs and off-chain tools:
- `SolanaDecoder::decode(bytes)` using `alloy-sol-types` `no_std`
- `sol!` macro type definitions for `SolanaHeader`, `SolanaIx`, `SolanaMeta`, `SolanaTxLeaf`
- `default-features = false` so it compiles for BPF target in Solana programs
- See Section 6 for full implementation

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

### 9.3 `solana_abi_encoding::solana_v1_encode`

```rust
// common/solana-abi-encoding/src/lib.rs
// ABI encoding using alloy::dyn_abi — mirrors the EVM V1 encoder.

use alloy::dyn_abi::DynSolValue;
use alloy::primitives::U256;
use solana_transaction_status::{
    EncodedTransactionWithStatusMeta,
    EncodedTransaction,
    UiMessage,
};

#[derive(thiserror::Error, Debug)]
pub enum EncodeError {
    #[error("Missing transaction meta")]
    MissingMeta,
    #[error("Failed to decode base58 pubkey: {0}")]
    Base58Decode(String),
    #[error("Transaction has no accounts")]
    NoAccounts,
    #[error("ABI encoding error: {0}")]
    AbiError(String),
}

/// ABI-encode a Solana transaction into the SolanaV1 leaf format.
/// Returns a DynSolValue::Tuple that can be .abi().to_vec() into bytes.
pub fn solana_v1_encode(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<DynSolValue, EncodeError> {
    let header_chunk = encode_header(tx)?;
    let instructions_chunk = encode_instructions(tx)?;
    let meta_chunk = encode_meta(tx)?;

    Ok(DynSolValue::Tuple(vec![
        DynSolValue::Uint(U256::from(0u8), 8), // version = 0 (SolanaV1)
        DynSolValue::Array(vec![
            DynSolValue::Bytes(header_chunk),
            DynSolValue::Bytes(instructions_chunk),
            DynSolValue::Bytes(meta_chunk),
        ]),
    ]))
}

fn decode_pubkey(s: &str) -> Result<[u8; 32], EncodeError> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| EncodeError::Base58Decode(e.to_string()))?;
    bytes.try_into()
        .map_err(|_| EncodeError::Base58Decode(format!("pubkey not 32 bytes: {s}")))
}

fn encode_header(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    // Parse account_keys, fee_payer, recent_blockhash from UiMessage
    // Then ABI-encode as (bytes32, bytes32, bytes32[])
    let (fee_payer, recent_blockhash, account_keys, _) = extract_message_fields(tx)?;

    let encoded = DynSolValue::Tuple(vec![
        DynSolValue::FixedBytes(alloy::primitives::FixedBytes::from(fee_payer), 32),
        DynSolValue::FixedBytes(alloy::primitives::FixedBytes::from(recent_blockhash), 32),
        DynSolValue::Array(
            account_keys.into_iter()
                .map(|k| DynSolValue::FixedBytes(alloy::primitives::FixedBytes::from(k), 32))
                .collect()
        ),
    ]);
    Ok(encoded.abi().to_vec())
}

fn encode_instructions(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    // For each instruction: (uint8 program_id_index, uint8[] account_indices, bytes data)
    let (_, _, _, instructions) = extract_message_fields(tx)?;

    let encoded = DynSolValue::Array(
        instructions.into_iter().map(|(prog_idx, acct_indices, data)| {
            DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::from(prog_idx), 8),
                DynSolValue::Array(
                    acct_indices.into_iter()
                        .map(|i| DynSolValue::Uint(U256::from(i), 8))
                        .collect()
                ),
                DynSolValue::Bytes(data),
            ])
        }).collect()
    );
    Ok(encoded.abi().to_vec())
}

fn encode_meta(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    let meta = tx.meta.as_ref().ok_or(EncodeError::MissingMeta)?;

    let log_messages: Vec<DynSolValue> = meta.log_messages
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|s| DynSolValue::Bytes(s.into_bytes()))
        .collect();

    let encoded = DynSolValue::Tuple(vec![
        DynSolValue::Bool(meta.err.is_some()),
        DynSolValue::Uint(U256::from(meta.fee), 64),
        DynSolValue::Array(
            meta.pre_balances.iter()
                .map(|b| DynSolValue::Uint(U256::from(*b), 64))
                .collect()
        ),
        DynSolValue::Array(
            meta.post_balances.iter()
                .map(|b| DynSolValue::Uint(U256::from(*b), 64))
                .collect()
        ),
        DynSolValue::Array(log_messages),
    ]);
    Ok(encoded.abi().to_vec())
}

fn extract_message_fields(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<([u8; 32], [u8; 32], Vec<[u8; 32]>, Vec<(u8, Vec<u8>, Vec<u8>)>), EncodeError> {
    // Handle both Binary and JSON-parsed transaction encodings
    match &tx.transaction {
        EncodedTransaction::Json(ui_tx) => {
            match &ui_tx.message {
                UiMessage::Raw(msg) => extract_from_raw_message(msg),
                UiMessage::Parsed(msg) => extract_from_parsed_message(msg),
            }
        }
        _ => todo!("handle base58/base64 encoded transactions")
    }
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

**Resolved (2026-04-24):** Primary target is **Solana programs**. `SolanaV1` uses **ABI encoding** — same as EVM `V1`. Decoded on-chain using `alloy-sol-types` `no_std` (BPF-compatible). Same bytes work on EVM contracts and Solana programs. See Sections 5 and 6.

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
- Fixture test: known Solana transaction → known ABI-encoded bytes (determinism)
- Round-trip: `solana_v1_encode` → `SolanaDecoder::decode` (ABI) → matches original fields
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
3. Fetch the block, ABI-encode it (SolanaV1), build Merkle root
4. ABI-decode with `SolanaDecoder` (Rust), assert fields match original transaction
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
