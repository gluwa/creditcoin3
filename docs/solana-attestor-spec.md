# Solana Attestor Support — Technical Specification

**Status:** Draft  
**Author:** Protocol Engineering  
**Target Repo:** `gluwa/creditcoin3`  
**Last Updated:** 2026-04-24

---

## Table of Contents

1. [Overview](#1-overview)
2. [Background: How Solana Works](#2-background-how-solana-works)
3. [Current EVM Pipeline (Reference)](#3-current-evm-pipeline-reference)
4. [What Changes for Solana](#4-what-changes-for-solana)
5. [ABI Encoding Design (SolanaV1)](#5-abi-encoding-design-solanav1)
6. [Proposed Architecture](#6-proposed-architecture)
7. [File-by-File Change Map](#7-file-by-file-change-map)
8. [Code Hints & Skeletons](#8-code-hints--skeletons)
9. [Open Questions](#9-open-questions)
10. [Testing Strategy](#10-testing-strategy)
11. [Appendix: Solana RPC Reference](#11-appendix-solana-rpc-reference)

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

This is the most design-critical component. The encoding must be:

1. **Deterministic** — same transaction always produces the same bytes
2. **ABI-compatible** — can be decoded by Solidity if needed (future on-chain verification)
3. **Complete** — captures enough data for a meaningful attestation
4. **Stable** — adding new Solana transaction versions must not break existing attestations

### 5.1 Structure Overview

The `V1` EVM encoding produces:

```
DynSolValue::Tuple([
    DynSolValue::Uint(type_id, 8),     // transaction type (Legacy=0, EIP1559=2, etc.)
    DynSolValue::Array(chunks),        // ABI-encoded sub-payloads
])
```

The `SolanaV1` encoding follows the same top-level shape for consistency with the Merkle tree and any future on-chain verifier:

```
DynSolValue::Tuple([
    DynSolValue::Uint(0, 8),          // version = 0 (SolanaV1, always 0 for now)
    DynSolValue::Array([
        Bytes(header_chunk),           // accounts + recent_blockhash + fee_payer
        Bytes(instructions_chunk),     // all instructions
        Bytes(meta_chunk),             // execution outcome (err, fee, balances, logs)
    ])
])
```

### 5.2 Chunk Definitions

**`header_chunk`** — ABI-encoded `Tuple`:
```
abi.encode(
    bytes32 fee_payer,          // account_keys[0], zero-padded pubkey
    bytes32 recent_blockhash,   // base58-decoded 32 bytes
    bytes32[] account_keys,     // all account pubkeys in order (padded to 32)
)
```

**`instructions_chunk`** — ABI-encoded `Tuple[]`, one per instruction:
```
abi.encode(
    Instruction[] {
        uint8    program_id_index,   // index into account_keys
        uint8[]  account_indices,    // indices of accounts used by this ix
        bytes    data,               // raw instruction data bytes
    }[]
)
```

**`meta_chunk`** — ABI-encoded `Tuple`:
```
abi.encode(
    bool     is_err,             // true if transaction failed
    uint64   fee,                // lamports paid
    uint64[] pre_balances,       // parallel to account_keys
    uint64[] post_balances,      // parallel to account_keys
    bytes[]  log_messages,       // UTF-8 program log lines
)
```

### 5.3 Why This Shape

- **`fee_payer` as `bytes32`** — consistent with EVM `address` being `bytes20`/padded; makes Solidity `abi.decode` trivial
- **`recent_blockhash` as `bytes32`** — already 32 bytes (SHA-256 output), directly usable
- **No signatures** — signatures are excluded from the encoding. They don't affect execution outcome and including them would make the leaf hash non-deterministic across signature variations of the same logical transaction (Solana allows multiple valid signature encodings for the same message)
- **`meta_chunk` included** — unlike EVM where the receipt is a separate fetch, in Solana the `meta` is always returned alongside the transaction. Including it means the Merkle root captures execution outcome, not just intent
- **Log messages as `bytes[]`** — UTF-8 strings, but encoded as `bytes` to avoid encoding ambiguity. Each log line is one element

### 5.4 Skipped Transactions

If a slot was skipped (no block), produce an **empty Merkle tree** — same as an empty EVM block. The Keccak Merkle tree already handles the empty case:

```rust
merkle::KeccakMerkleTree::new(&[]).root() // → some canonical empty root
```

### 5.5 Version Future-Proofing

The `uint8` version field in position 0 allows a `SolanaV2` encoding to be introduced later (e.g., for versioned transactions, Address Lookup Tables) without changing the outer shape. A verifier can switch decode logic based on this byte.

---

## 6. Proposed Architecture

### 6.1 Option A: Trait Abstraction (Recommended)

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

### 6.2 Option B: Parallel Crates (Simpler)

Keep `common/streams/eth` as-is. Add `common/streams/solana` as a parallel crate with `SolanaStreamRoots` and `SolanaStreamTip` that mirror the EVM versions but use `solana::Client`.

**Pros:** Zero risk to existing EVM path, easy to implement  
**Cons:** Code duplication in stream logic; reconnect/backoff logic needs to be maintained in two places

### 6.3 Recommendation

**Start with Option B** to ship faster and avoid breaking the EVM path. Refactor to Option A in a follow-up PR once both paths are proven stable.

### 6.4 Attestor Entrypoint Branching

`attestor/attestor/src/lib.rs` currently creates `eth::Client` unconditionally. This must become conditional:

```
match supported_chain.chain_encoding {
    ChainEncodingVersion::V1     => run_evm_path(config, supported_chain).await,
    ChainEncodingVersion::SolanaV1 => run_solana_path(config, supported_chain).await,
}
```

Each path manages its own client init, chain ID validation, stream construction, and maturity strategy interpretation.

---

## 7. File-by-File Change Map

### 7.1 `primitives/attestor/src/lib.rs`

**Change:** Add `SolanaV1 = 2` to `ChainEncodingVersion`.

```rust
pub enum ChainEncodingVersion {
    V1 = 1,
    SolanaV1 = 2,
}
```

This enum is stored on-chain via SCALE codec. Adding a new variant is **backwards-compatible** (existing encoded `V1 = 1` decodes correctly). No migration needed for the enum itself, only if `SupportedChain` storage layout changes.

Also update the `From<ChainEncodingVersion>` for `usc_abi_encoding::common::EncodingVersion` — this only applies to `V1`; `SolanaV1` routes to the new Solana encoder, not through `EncodingVersion`.

### 7.2 `primitives/supported-chains/src/lib.rs`

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

### 7.3 New Crate: `common/solana`

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

### 7.4 New Crate: `solana-abi-encoding` (or extend `usc-abi-encoding`)

**Recommended:** New crate `common/solana-abi-encoding` with no dependency on `alloy::rpc::types::Transaction`. The existing `usc-abi-encoding` is tightly coupled to alloy types.

**Contents:**
- `pub fn solana_v1_encode(tx: &EncodedTransactionWithStatusMeta) -> Result<DynSolValue, EncodeError>`
- Internal helpers: `encode_header`, `encode_instructions`, `encode_meta`

### 7.5 New Crate: `common/streams/solana` (Option B)

- `SolanaStreamRoots` — mirrors `eth::StreamRoots`, uses `solana::Client`
- `SolanaStreamTip` — mirrors `eth::StreamTip`, uses `SlotInfo.root` for finalized tip

### 7.6 `attestor/attestor/src/lib.rs`

**Change:** Branch on `supported_chain.chain_encoding` after fetching the chain config. Extract the EVM-specific client init into a sub-function or module. Add Solana path.

Key changes:
1. Config struct needs a `url_solana: Option<RpcSecret>` alongside `url_eth`
2. `wait_for_endpoints` must check the correct URL based on chain encoding
3. Chain ID validation differs: EVM uses `client_eth.chain_id() == supported_chain.chain_id`; Solana uses genesis-hash-derived ID
4. `StreamRoots` and `StreamTip` wiring becomes conditional

### 7.7 `runtime/src/migrations.rs`

**Likely not needed** for just adding a new `ChainEncodingVersion` variant (SCALE enum encoding is by discriminant, and `SolanaV1 = 2` is a new discriminant). However, if `SupportedChain` itself grows a new field, a migration IS needed.

No new fields are proposed in this spec — only new variants on existing enums.

### 7.8 Chainspecs

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

## 8. Code Hints & Skeletons

### 8.1 `solana::Client`

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

### 8.2 `SolanaOrderedBlock` and `SolanaTxItem`

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

### 8.3 `solana_abi_encoding::solana_v1_encode`

```rust
// common/solana-abi-encoding/src/lib.rs

use alloy::dyn_abi::DynSolValue;
use solana_transaction_status::{
    EncodedTransactionWithStatusMeta, UiTransactionStatusMeta,
};

#[derive(thiserror::Error, Debug)]
pub enum EncodeError {
    #[error("Missing transaction meta")]
    MissingMeta,
    #[error("Failed to decode base64: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("Failed to decode base58: {0}")]
    Base58Decode(String),
}

pub fn solana_v1_encode(
    tx: &EncodedTransactionWithStatusMeta,
) -> Result<DynSolValue, EncodeError> {
    let header_chunk = encode_header(tx)?;
    let instructions_chunk = encode_instructions(tx)?;
    let meta_chunk = encode_meta(tx)?;

    Ok(DynSolValue::Tuple(vec![
        DynSolValue::Uint(alloy::primitives::U256::from(0u8), 8), // version = 0
        DynSolValue::Array(vec![
            DynSolValue::Bytes(header_chunk),
            DynSolValue::Bytes(instructions_chunk),
            DynSolValue::Bytes(meta_chunk),
        ]),
    ]))
}

fn encode_header(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    // Extract UiMessage from the transaction
    // fee_payer = account_keys[0]
    // recent_blockhash = 32-byte hash
    // account_keys = all pubkeys

    // ... parse UiTransaction/UiMessage
    // ... base58-decode pubkeys into [u8; 32]
    // ... ABI-encode as (bytes32, bytes32, bytes32[])

    todo!("implement header encoding")
}

fn encode_instructions(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    // For each instruction: (uint8 program_id_index, uint8[] accounts, bytes data)
    // ABI-encode as array of tuples

    todo!("implement instruction encoding")
}

fn encode_meta(tx: &EncodedTransactionWithStatusMeta) -> Result<Vec<u8>, EncodeError> {
    let meta = tx.meta.as_ref().ok_or(EncodeError::MissingMeta)?;

    // is_err: meta.err.is_some()
    // fee: meta.fee
    // pre_balances: meta.pre_balances
    // post_balances: meta.post_balances
    // log_messages: meta.log_messages (Vec<String> → Vec<bytes>)

    // ABI-encode as (bool, uint64, uint64[], uint64[], bytes[])

    todo!("implement meta encoding")
}
```

### 8.4 `SolanaStreamTip` (Option B)

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

### 8.5 `SolanaStreamRoots`

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

### 8.6 Attestor Entrypoint Branch

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

## 9. Open Questions

These must be resolved before implementation begins.

### Q1: On-chain block proof verification needed?

**Question:** Does `BlockProver.sol` or any on-chain precompile need to verify Solana transaction data? Or do attestors only attest (produce a Merkle root) without requiring on-chain proof of individual transactions?

**Impact:** If on-chain verification is needed, the `SolanaV1` encoding MUST be decodable by Solidity, which constrains the encoding choices significantly. If attestor-only, encoding just needs to be deterministic.

**Current assumption:** Attestor-only (no on-chain Solana tx verification). Encoding is deterministic but not necessarily Solidity-verifiable today.

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

## 10. Testing Strategy

### 10.1 Unit Tests

**`solana-abi-encoding`:**
- Fixture test: known Solana transaction bytes → known ABI-encoded bytes (determinism)
- Round-trip: encode → decode (if adding decoder) matches original
- Empty transaction list → empty bytes (not crash)
- Failed transaction (is_err = true) encoded correctly

**`solana::SolanaOrderedBlock`:**
- `from_confirmed_block` with mock block data
- `empty(slot)` produces zero-length items

**`genesis_hash_to_chain_id`:**
- Known genesis hashes → expected u64 values

### 10.2 Integration Tests

**Against `solana-test-validator`:**
1. Start local validator
2. Send a few transactions
3. Fetch the block, encode it, build Merkle root
4. Compare root to a known-good value (generated once and frozen)

```bash
# Setup
solana-test-validator --reset &
sleep 5

# Send test tx
solana transfer --allow-unfunded-recipient <addr> 0.001 --keypair test-wallet.json

# Run integration test
cargo test --test solana_encoding_integration
```

### 10.3 End-to-End Test

Run the attestor against a local Solana test validator + local CC3 devnet:

1. Register a test `SupportedChain` with `chain_encoding: SolanaV1` on a local CC3 runtime
2. Start one attestor pointing at the test validator
3. Verify attestations appear on CC3 with correct Merkle roots
4. Check digest chain continuity (`prev_digest` links correctly)

### 10.4 Encoding Stability Test

Add a test that serializes a known transaction using `SolanaV1`, stores the hex output as a fixture, and asserts it never changes across builds. This prevents accidental encoding drift.

---

## 11. Appendix: Solana RPC Reference

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
