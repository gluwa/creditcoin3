# message-relayer crate — initial PoC plan

## Context

`relayer-poc.pdf` (15 pages, repo root) describes the **USC Write-Ability Relayer**: an off-chain service that delivers cross-chain messages emitted by the Creditcoin L1 `Outbox` to destination-chain `Inbox` contracts. The relayer is **not part of the security model** — it transports evidence (attester signatures gossiped on libp2p) and pays destination gas. Validity comes from `attester quorum + inbox math`, not from the relayer.

This crate is the Phase-1 PoC. Goal: a single binary that for one or more `(creditcoin_chain_key → destination_chain)` routes will (1) discover finalized `MessagePublished` events on Creditcoin, (2) snoop attester votes on the existing P2P mesh, (3) aggregate ECDSA signatures up to a `2/3+1` threshold, and (4) submit `Inbox.deliverMessage(...)` on the destination chain. Phasing/quoting/profit-sharing (L11–L12) and acknowledgment proofs (PDF §8) are explicitly out of scope.

The user asked for "good enough for starting", so the plan favors pragmatic copies of existing workspace patterns over architectural purity.

## Inspiration crates (already audited)

- `proof-gen-api-server/` — single workspace member, `bin/server.rs` + `src/{lib.rs, config.rs, events/, prom/, services/, networking/}`, multi-chain via YAML, `dotenvy` for `.env`, `Server::new(config)` then `Server::run()`, top-level `tokio::spawn` for the CC3 event subscriber, `prometheus_client` metrics with typed labels, `axum` for `/metrics` + `/health`. Reference: `proof-gen-api-server/src/lib.rs:35`, `:274`, `:308`.
- `attestor/attestor/` — libp2p stack we copy for read-only subscription: `gossipsub` (Strict + Signed), `kad`, `identify`, `mdns` (toggle), `ping`, TCP+QUIC+DNS, ed25519 keypair derived from BIP39/raw-seed. Reference: `attestor/attestor/src/worker/p2p/mod.rs:140`, `behavior.rs:1`, `protocols.rs:1`, `secret.rs:13`. We do **not** copy attestor's `Worker` trait + `CancellationMonitor` — that pattern exists for key isolation, irrelevant for a transport process; we use `tokio::spawn` like proof-gen.
- `common/eth/src/evm/block_prover.rs:36` — canonical `alloy::sol!` macro with JSON ABI for inline binding generation.
- `common/cc-client` — read-only `Client::new_read_only(url)`, `subscribe_events_chains(&[ChainKey])` for finalized-event streaming, `get_supported_chain(chain_key)` for chain_id lookup.
- `common/eth::Client::new(url, private_key)` — provides ws/http providers, signer, and chain_id.

## Crate layout

New workspace member at `/home/gluwa/Repos/creditcoin3/message-relayer/`. Add `"message-relayer"` to `members` in `/home/gluwa/Repos/creditcoin3/Cargo.toml`.

```
message-relayer/
├── Cargo.toml
├── README.md
├── bin/
│   └── relayer.rs               # clap entrypoint, dotenvy, tracing init, Server::new + .run()
├── config.example.yaml          # documented sample multi-route config
├── contracts/
│   ├── outbox.json              # ABI artefact (placeholder until production contract lands)
│   ├── inbox.json
│   └── vote_validator.json
└── src/
    ├── lib.rs                   # `Server` struct, `pub mod` re-exports, `shutdown_signal()`
    ├── config.rs                # ChainRoute + Config + YAML loader (proof-gen pattern)
    ├── abi.rs                   # `alloy::sol!` for IOutbox, IInbox, IVoteValidator
    ├── hash.rs                  # messageHash builder + golden-vector tests
    ├── events/
    │   └── mod.rs               # MessagePublished poller/subscriber per route
    ├── p2p/
    │   ├── mod.rs               # WorkerP2P (libp2p swarm) — tokio::spawn'ed task
    │   ├── behavior.rs          # NetworkBehaviour clone of attestor (read-only subscribe)
    │   ├── envelope.rs          # MessageVote { message_id, message_hash, signature, ... } + SCALE codec
    │   └── protocols.rs         # IDENTIFY/KADEMLIA constants (own namespace, e.g. "/gluwa/relayer/...")
    ├── pool/
    │   └── mod.rs               # VotePool: HashMap<H256, BTreeMap<Address, [u8;65]>> + LRU/TTL
    ├── delivery/
    │   ├── mod.rs               # per-route DeliveryWorker — eth_call simulate, send tx, watch receipt
    │   └── encode.rs            # `votes = abi.encode(bytes[] memory signatures)` + sort-by-signer
    └── prom/
        └── mod.rs               # prometheus_client metrics; copies proof-gen layout
```

The minimal HTTP surface is `/metrics` + `/health` only — folded into `prom/` rather than a separate `networking/` module.

## Module design (concrete enough to start coding)

### `config.rs`

```rust
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub p2p: P2pConfig,
    pub vote_cache: VoteCacheConfig,
    pub delivery: DeliveryConfig,
    pub routes: Vec<ChainRoute>,
}

pub struct ChainRoute {
    pub chain_key: u64,                        // Creditcoin ChainKey, also gossipsub topic prefix
    pub creditcoin_chain_id: u64,              // bound for messageHash; cross-checked against CC3
    pub outbox_address: Option<alloy::primitives::Address>, // None = factory resolution
    pub destination_rpc_url: String,
    pub inbox_address: alloy::primitives::Address,
    pub signer_key: Option<String>,            // hex / mnemonic; required to deliver, optional read-only
    pub block_confirmation_depth: u64,
    pub attester_set: AttesterSet,             // allowlist source
    pub threshold_override: Option<u32>,
}

pub enum AttesterSet {
    Static(Vec<alloy::primitives::Address>),
    OnChain { source: AttesterSource },        // EVM contract or CC3 chain-key lookup
}

pub struct P2pConfig {
    pub port: u16, pub public_addr: Option<String>,
    pub boot_nodes: Vec<libp2p::Multiaddr>,
    pub no_mdns: bool,
    pub identity: Option<String>,              // BIP39 mnemonic / 0x-hex seed; ephemeral if None
}

pub struct VoteCacheConfig { pub ttl_seconds: u64, pub max_messages: usize }
pub struct DeliveryConfig { pub simulate_before_send: bool, pub max_retries: u32, pub gas_multiplier: f64 }
```

YAML loader follows `proof-gen-api-server/src/config.rs` structure (`ConfigFile` with `serde::Deserialize`, `into_config()`, dedup of `chain_key`s). CLI: a `--config <path>` flag plus a `--single-route` quickstart flag that builds a one-route Config from CLI args (CC3/dest URLs + key + inbox addr) — explicit flag avoids proof-gen's legacy-CLI ambiguity.

### `abi.rs`

```rust
alloy::sol! {
    #[sol(rpc)] interface IOutbox, "contracts/outbox.json"
}
alloy::sol! {
    #[sol(rpc)] interface IInbox, "contracts/inbox.json"
}
alloy::sol! {
    #[sol(rpc)] interface IVoteValidator, "contracts/vote_validator.json"
}
```

The placeholder JSON files contain the function signatures we need today (`MessagePublished` event, `deliverMessage`, `validateVotes`, `retryPendingMessage`); swap them for production artefacts when contracts land. Mirrors `common/eth/src/evm/block_prover.rs:36`.

### `hash.rs`

```rust
/// keccak256(abi.encode(messageId, emitterAddress, destinationChainKey, creditcoinChainId, payload))
pub fn message_hash(
    message_id: B256,
    emitter: Address,
    destination_chain_key: u64,
    creditcoin_chain_id: u64,
    payload: &[u8],
) -> B256
```

Implemented via `alloy::sol_types::SolValue::abi_encode` so encoding is bit-exact with Solidity (PDF §5.2). Golden-vector tests (T1) compare against fixtures generated from a reference Solidity contract.

### `events/mod.rs`

Per-route Outbox watcher. Implementation:
1. Resolve outbox: if `route.outbox_address` is `Some`, use it directly; else call a factory (planned `bytes32` chain-key lookup) — see *Factory resolution* below.
2. Use `eth::Client::subscribe()`-style WS subscription to the `MessagePublished` topic on the Outbox, with a fallback `eth_getLogs` poller (use `block_confirmation_depth` to avoid reorg-prone heads — same pattern as `proof-gen-api-server/src/lib.rs:232`).
3. Validate the event shape (scale-encoded fields per attester spec), compute `messageHash`, push `IndexedMessage` into a shared `Arc<RwLock<HashMap<H256, IndexedMessage>>>` (the **chain-first allowlist** per PDF §6.2 — votes for unindexed `messageHash`es are dropped).

### `p2p/mod.rs`

Single libp2p swarm shared by all routes (one mesh; topics differ by `chain_key`). Build the swarm exactly like `attestor/attestor/src/worker/p2p/mod.rs:140` but in a `tokio::spawn`'ed task instead of an OS thread:

- TCP + QUIC + DNS transports
- `gossipsub` (`MessageAuthenticity::Signed`, `ValidationMode::Strict`, `validate_messages()`)
- `kad`, `identify`, `mdns` (toggle), `ping`, connection-limits
- For each configured route, subscribe to topic `format!("{}/message-votes/v1", route.chain_key)`
- Use namespaced protocol ids: `/gluwa/relayer-id/1.0.0`, `/gluwa/relayer-kad/1.0.0` (do **not** reuse attestor's `/gluwa/id/1.0.0` — same mesh but different identity domain).

`p2p/envelope.rs` defines:

```rust
#[derive(Debug, Clone, Encode, Decode)]
pub struct MessageVote {
    pub chain_key: u64,
    pub message_id: B256,
    pub message_hash: B256,
    pub signer: Address,
    pub signature: [u8; 65],   // r || s || v
}
```

Decoded with `parity_scale_codec::Decode` (matches attestor codec). When the canonical attester envelope ships, replace this with a `From` impl and delete the local definition.

### `pool/mod.rs`

```rust
pub struct VotePool {
    by_message: HashMap<B256, MessageState>,   // key = message_hash
    cache: lru::LruCache<B256, ()>,            // TTL/max_messages enforcement (PDF §9)
}
struct MessageState {
    indexed: IndexedMessage,                   // From events/, populated lazily
    signers: BTreeMap<Address, [u8; 65]>,      // dedup + deterministic ordering
    delivered: bool,
}
```

API:
- `note_indexed(IndexedMessage)` — chain-first allowlist
- `try_insert_vote(MessageVote, &AttesterSet) -> InsertOutcome` — apply PDF §6.2 checks (codec, allowlist, `ecrecover`, signer ∈ allowlist, dedup) and increment metrics
- `ready_for_delivery(threshold) -> Option<DeliveryJob>` — fires when `unique_signers >= threshold`

Threshold is computed from `AttesterSet` length (`2*N/3 + 1`, matching PDF §6.3) unless `threshold_override` is set.

### `delivery/mod.rs`

Per-route `tokio::spawn` worker:

1. Receive `DeliveryJob { message_id, emitter, payload, votes_calldata }` over an `mpsc::Receiver`.
2. (PDF §7.2) `eth_call` simulate `Inbox.deliverMessage(...)` to catch `validateVotes` revert before paying gas.
3. Submit transaction via `eth::Client::get_wallet_ws_provider()` signer; serial nonce (one wallet per route is enough for the PoC; throughput optimisation is post-PoC).
4. Watch receipt; on success increment `messages_delivered`, on revert classify (`AlreadyValidated` → idempotent success, `Pending` → schedule `retryPendingMessage`, other → log + backoff).
5. Multiplicative backoff with jitter; bound by `DeliveryConfig::max_retries`.

`delivery/encode.rs` builds `votes = abi.encode(bytes[] memory signatures)` (PDF §6.4): collect signatures from `MessageState.signers`, sorted by signer address (deterministic txs), encode via `Vec::<Bytes>::abi_encode_params`.

### Factory resolution (D7 follow-up)

Implement now, behind a small `outbox_factory` resolver that takes a `bytes32` chain-key and returns the `Outbox` address. Until the production factory contract is available the resolver short-circuits to the configured `outbox_address`, and the production code path is exercised by a `#[cfg(test)]` mock. This avoids a `TODO: factory` rotting across PRs.

### `prom/mod.rs`

Copy `proof-gen-api-server/src/prom/mod.rs` shape. Counters/histograms required by PDF §10:
- `relayer_messages_indexed_total{chain_key}`
- `relayer_votes_received_total{chain_key, accept|reject|ignore}`
- `relayer_votes_per_message` (histogram)
- `relayer_deliver_tx_total{chain_key, submitted|succeeded|reverted|already_validated}`
- `relayer_time_to_threshold_seconds`, `relayer_time_to_deliver_seconds`
- `relayer_p2p_peer_count{chain_key}`, `relayer_pool_messages_pending`
- Plus the standard CPU/RAM/threads gauges from proof-gen.

`/metrics` and `/health` are exposed via a tiny `axum::Router` started from `Server::run()` — does not warrant a full `networking/` module.

## Server lifecycle (`lib.rs`)

```rust
pub struct Server { config: Config, cc3_client: Arc<CcClient>, prom: Arc<RelayerMetrics> }

impl Server {
    pub async fn new(config: Config) -> Result<Self> { /* connect CC3, validate routes, build prom */ }
    pub async fn run(self) -> Result<()> {
        // 1. Build shared VotePool
        // 2. Spawn libp2p WorkerP2P (subscribes to all routes' topics)
        // 3. For each route: spawn events::watch_outbox + delivery::run
        // 4. Spawn axum /metrics + /health server
        // 5. select! on shutdown_signal() vs the spawned tasks (proof-gen pattern)
    }
}
```

Cross-task coordination via `tokio::sync::mpsc::Sender<MessageVote>` (P2P → Pool), `mpsc::Sender<IndexedMessage>` (Outbox watcher → Pool), and `mpsc::Sender<DeliveryJob>` (Pool → per-route DeliveryWorker). The Pool itself runs in its own task so locking stays simple. Shutdown signal mirrors `proof-gen-api-server/src/lib.rs:353`.

## Cargo.toml dependencies

Reuse workspace deps: `alloy` (full), `anyhow`, `axum`, `clap`, `dotenvy`, `futures`, `libp2p` (tokio/macros/ed25519/dns/quic/tcp/noise/yamux/identify/ping/mdns/kad/gossipsub), `parity-scale-codec`, `prometheus-client`, `serde`, `serde_yaml`, `sysinfo`, `thiserror`, `tokio`, `tower-http`, `tracing`, `tracing-subscriber`, `url`, `zeroize`. Workspace path deps: `cc-client`, `eth`, `attestor-primitives`, `supported-chains-primitives`, `usc-abi-encoding`, `builder` (only if needed for typed config — proof-gen-style plain structs likely sufficient). New dev-deps: `wiremock`, `alloy-node-bindings` (anvil), `parameterized`.

Top of file mirrors `proof-gen-api-server/Cargo.toml:1` for clippy lints, `[lib]`/`[bin]` paths, `[features]` (`integration-tests`), and `[package.metadata.release] release = false`.

## Phased PR roadmap (six PRs, each independently merge-able)

| PR | Scope | Demonstrates |
|---|---|---|
| **PR1** | Skeleton: workspace member, `Cargo.toml`, `bin/relayer.rs` parsing config, `Config` + YAML loader, empty `Server::run()` that spawns `/metrics` and exits cleanly. CI green. | L0, L1 |
| **PR2** | Outbox discovery: `abi.rs` (Outbox only), `events/`, `hash.rs` with golden-vector tests against fixtures. Indexed messages logged but not yet acted on. | L2, L3, L4, **T1** |
| **PR3** | P2P subscriber: `p2p/` with libp2p swarm, gossipsub subscription, `MessageVote` decode. Votes are logged + counted in metrics; aggregation not yet wired. | L5 |
| **PR4** | Vote pool + threshold: `pool/`, vote validation (codec, ecrecover, allowlist, dedup), votes calldata encoding (`delivery/encode.rs`). `DeliveryJob` produced but not submitted. | L6, L7 |
| **PR5** | Delivery happy-path: `Inbox` ABI, simulate-then-send via `eth::Client`, success path metrics. Single route end-to-end on local Anvil. | L8, **T2** |
| **PR6** | Hardening: `MessagePending` + `retryPendingMessage` handling, idempotent already-validated path, backoff/jitter, vote cache TTL/LRU, abuse tests, race tests. | L9, L10, **T3, T4** |

Slicing rationale: each PR keeps reviewer scope to one concern. PR2 ends with a verifiable hash builder before any P2P code lands; PR3 ships a subscriber that compiles and runs without aggregation; PR5 makes the binary end-to-end before hardening lands.

## Test plan (PDF §11.6)

- **T1 (golden hash + votes encoding):** `tests/golden_hash.rs` with vectors generated from a reference Solidity. `tests/golden_votes_encoding.rs` reconstructing `validateVotes` calldata and asserting bytes match `abi.encode(bytes[])`. Runs in CI without Anvil.
- **T2 (E2E):** `tests/e2e_anvil.rs` behind `--features integration-tests`. Boots Anvil + dummy Outbox/Inbox + a small libp2p mesh (subscribe-only relayer + a fixture publisher). Asserts `deliverMessage` lands and emits `MessageDelivered`. Mirrors `proof-gen-api-server/tests/`.
- **T3 (abuse):** Property-style test (`proptest` or `parameterized`) that floods the gossipsub topic with malformed envelopes / duplicate signers / unknown signers and asserts pool memory stays bounded by `vote_cache.max_messages` and no `DeliveryJob` is produced.
- **T4 (race):** Spawn two relayer instances against the same Anvil; assert exactly one `MessageDelivered`, the other observes `AlreadyValidated` and treats it as success.

CI hooks: extend the existing workspace `cargo test` job to include `-p message-relayer`. Integration tests opt-in via the `integration-tests` feature exactly like proof-gen does.

## Critical files to touch / create

- **Add** `/home/gluwa/Repos/creditcoin3/message-relayer/Cargo.toml`
- **Add** `/home/gluwa/Repos/creditcoin3/message-relayer/bin/relayer.rs`
- **Add** `/home/gluwa/Repos/creditcoin3/message-relayer/src/{lib.rs,config.rs,hash.rs,abi.rs,events/mod.rs,p2p/{mod.rs,behavior.rs,envelope.rs,protocols.rs},pool/mod.rs,delivery/{mod.rs,encode.rs},prom/mod.rs}`
- **Add** `/home/gluwa/Repos/creditcoin3/message-relayer/contracts/{outbox,inbox,vote_validator}.json`
- **Add** `/home/gluwa/Repos/creditcoin3/message-relayer/config.example.yaml`
- **Edit** `/home/gluwa/Repos/creditcoin3/Cargo.toml` — append `"message-relayer"` to `members`. Optionally add a `message-relayer = { path = "message-relayer" }` workspace dep entry to mirror house style.

No edits to existing crates required for the PoC scope. The relayer is purely additive.

## Open questions / explicit assumptions (flag these in PR descriptions)

1. **Vote envelope canonical schema** — `MessageVote` codec is a best-effort interpretation of PDF §6.1; will need a one-line swap when the attester write-ability work merges its envelope type. Tracking comment in `p2p/envelope.rs`.
2. **Attester set source** — PoC supports only static allowlist (Phase 1). On-chain `IVoteValidator` introspection is wired through `AttesterSet::OnChain { source }` but the resolver is a stub (returns config'd allowlist) until the validator contract ships. Equivalent to PDF Open Question 5.
3. **Outbox factory** — placeholder resolver, returns `route.outbox_address` directly; tracked next to PDF L2.
4. **Signer alignment** — assumes attesters publish 65-byte ECDSA over the EVM `messageHash`; if attesters use a different scheme (BLS over a different msg) the PoC will fail at the `ecrecover` step and we revisit (PDF §6.5).
5. **Threshold formula** — `2N/3 + 1` matching PDF §6.3 / IVoteValidator default. `threshold_override` exists for tests.
6. **Phase-2 hooks** — `validateAndCollectFee` quoter integration and acknowledgment proofs are deliberately deferred; not instrumented in metrics names that would later collide.

## Verification

- `cargo build -p message-relayer` from workspace root must succeed at every PR boundary.
- `cargo clippy -p message-relayer --all-targets` must pass (workspace runs `-D warnings`).
- `cargo test -p message-relayer` (golden hash, votes-encoding, pool unit tests) must pass on every PR.
- `cargo test -p message-relayer --features integration-tests` must pass on PR5+ (E2E, race, abuse).
- Manual: copy `config.example.yaml`, point at a local CC3 node + Anvil + a single attester fixture, run `cargo run -p message-relayer -- --config relayer.yaml`, observe a `MessagePublished` → P2P vote → `deliverMessage` flow end-to-end. Check `/metrics` for the counters listed in §10 and `/health` returns 200.
- CI: add `-p message-relayer` to existing build/test/clippy matrices in `.github/workflows/` (no new pipelines).

## What is *not* in this plan

- No cross-crate refactors: existing `attestor`, `proof-gen-api-server`, `cc-client`, `eth` are untouched.
- No new pallet/runtime work: the Outbox/Inbox contracts are EVM and live outside the runtime crate.
- No production-grade key management: BIP39/hex-seed via config, like attestor; HSM/remote signer is a follow-up.
- No quoter/fee path, no acknowledgment proof submission, no profit distribution — explicitly post-PoC.
