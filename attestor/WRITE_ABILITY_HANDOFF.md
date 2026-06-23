# USC Write-Ability — Attestor Handoff

**Branch:** `attestor_v2_writability` · **Commit:** `7dd190456` (`feat(attestor): USC write-ability message attestation`)
**Status:** attestor half complete + tested (builds clean, clippy/fmt/taplo/machete clean, all unit/integration tests pass; anvil e2e passes). Cross-chain go-live blocked on 3 colleague-owned pieces + an ABI unification (see §3).

Message attestation makes the attestor a **message validator**: watch the Creditcoin L1 Outbox → sign the
canonical `messageHash` (ECDSA) → gossip a vote on `{chain_key}/message-votes/v1` (the **existing** p2p swarm,
new topic only). Relayers consume votes and deliver; the attestor never relays. Disabled by default.

---

## 1. Done ✅ (this commit)

- [x] **Shared crate `common/write-ability`** — `hash`, `envelope` (`MessageVote`), `abi` (`IOutbox`/`IInbox`/`IOutboxFactory`/`IChainInfo`/`IVoteValidator`), `protocol` (topic + `chain_key_to_bytes32`). `message-relayer` re-exports all four → producer/consumer can't diverge.
- [x] **A1 Config** — `tasks/write_ability/config.rs`; opt-in, disabled by default.
- [x] **A2 Resolver** — chain-info precompile (`0x…0fD3`, PR #873) → `IOutboxFactory.getOutbox` → assert `Outbox.chainKey()`; config fallbacks.
- [x] **A3 Listener** — `MessagePublished` poll + confirmation-depth finality gate.
- [x] **A5 Signing** — EVM secp256k1 key domain-derived from the attestor seed; raw-hash sign (no EIP-191); recovery matches relayer.
- [x] **A6 Gossip** — new topic on the existing swarm (`tasks/p2p/mod.rs` dispatch-by-topic); shared `MessageVote` envelope.
- [x] **A7 Aggregator** — unique signers, 2N/3+1, dedup.
- [x] **A11 Anti-abuse** — hard validation (decode + recover + signer ∈ set), chain-first allowlist, `max_tracked` + TTL + LRU.
- [x] **A9 Metrics / A10 Wiring** — `MessageVote` counter; spawned in `lib.rs` JoinSet via `Shared.message_votes` + publish channel.
- [x] **CLI/config** — `--message-attestation`, `--cc3-eth-url`, `ATTESTOR_*` env, documented `write_ability:` block in `attestor/config.yaml`.
- [x] **Tests** — hash golden vectors, signing round-trip, aggregator (threshold/dedup/cap/TTL), ingest abuse (forged signer, non-attester, unindexed, garbage, wrong-chain, duplicate), **anvil e2e** (`tests/e2e_anvil.rs`, `#[ignore]`).
- [x] Fixed pre-existing `message-relayer` build breakage + clippy warnings.

## 2. Attestor follow-ups ⏳ (same owner — hardening)

- [ ] **`AttesterSet::OnChainValidator`** — currently only `Static` works; reading `IVoteValidator.attesters()` needs a destination-chain RPC (bails with a clear log today).
- [ ] **Probabilistic finality when the gadget is paused** (research §6.8) — today fixed confirmation-depth only.
- [ ] **GossipSub peer-scoring + per-peer rate limits** for the message-vote topic (defaults used today).
- [ ] **`getOutbox` → `address(0)` backoff** + subscribe to `OutboxCreated` (today: fail-fast).
- [ ] **Publish-before-mesh** — gate/buffer message-vote publish until the topic has mesh peers (today best-effort; a too-early publish is dropped).
- [ ] **Vote persistence across restart** (SQLite/RocksDB) — in-memory by design for PoC.

## 3. Colleague-owned blockers 🚧 (required for live cross-chain)

- [ ] **Runtime `SupportedChain` write-ability fields + migration + RPC** (R1–R3): `write_ability_chain_key` (bytes32) + `message_attestation_enabled`, exposed via `get_supported_chain`. *(`outbox_factory_address` already shipped in PR #873.)* — attestor uses config until then.
- [ ] **`OutboxFactory` contract** (`getOutbox(bytes32)`) deployed + factory-created Outbox. — use `outbox_address` config override until then.
- [ ] **Production `EOAValidator`** replacing PoC `DummyVoteValidator` (accepts everything) + a process to keep its on-chain attester address set in sync with attestors' derived EVM addresses.
- [ ] **⚠️ ABI unification** — PoC `SimpleOutbox.sol` emits `MessagePublished(bytes32 messageId, bytes32 emitterAddress, bool requiresAck, bytes payload)` but the attestor/relayer `IOutbox` binding expects `(bytes32 messageId, address emitterAddress, bytes payload)`. **Must be reconciled** or the attestor won't decode real Outbox logs.

## 4. End-to-end / infra 🧪

- [ ] **Full zombienet binary e2e** — N real attestors + CC3 node + Outbox, votes over **real gossip** → quorum → relayer delivers to a real Inbox. (Current anvil e2e covers EVM→hash→sign→ingest→quorum but skips the gossip hop + full attestor startup.)
- [ ] **Relayer↔attestor live interop** — relayer's `e2e_anvil.rs` is still an `unimplemented!()` stub.
- [ ] **CI lane with foundry** to actually run the `#[ignore]`d anvil e2e.
- [ ] **Golden vector vs the deployed `validateVotes`** once the real validator exists.
- [ ] Operator runbook (env vars, topic, the boot-logged signer address to register) — research D2.

---

## How to enable (once §3 lands, or with config overrides for PoC)

```yaml
# attestor/config.yaml
write_ability:
  enabled: true
  cc3_eth_url: "ws://<creditcoin-evm-rpc>"
  attesters: ["0x<attester-evm-addr>", ...]   # include your own boot-logged signer address
  # PoC overrides until the factory/runtime fields exist:
  # outbox_address: "0x..."        # skip factory lookup
  # write_ability_chain_key: "0x..."
```
CLI equivalent: `--message-attestation --cc3-eth-url ws://… ` (env: `ATTESTOR_MESSAGE_ATTESTATION`, `ATTESTOR_CC3_ETH_URL`).
On boot the attestor logs its **derived EVM signer address** — register it in the `EOAValidator` and list it in `attesters`.

## Run the tests

```bash
cargo test -p write-ability -p attestor -p message-relayer   # unit + integration
cargo test -p attestor --test e2e_anvil -- --ignored          # anvil e2e (needs foundry)
```

## Key files

| Path | What |
|------|------|
| `common/write-ability/` | shared hash / envelope / abi / protocol (relayer re-exports) |
| `attestor/attestor/src/tasks/write_ability/` | config, resolver, listener, signing, aggregator, ingest, mod |
| `attestor/attestor/src/tasks/p2p/mod.rs` | topic subscribe + dispatch + publish (piggyback) |
| `attestor/attestor/src/shared.rs` | `message_votes: Option<Arc<MessageVoteState>>` |
| `attestor/attestor/src/main.rs` | CLI/config (`build_write_ability`) |
| `attestor/attestor/tests/e2e_anvil.rs` | anvil end-to-end |
| `attestor/config.yaml` | documented `write_ability:` section |
