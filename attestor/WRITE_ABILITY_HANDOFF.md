# USC Write-Ability — Attestor Handoff (context brief for an AI assistant)

> **How to use this doc:** paste it into a fresh Claude/agent session on another machine to onboard
> it onto this work. It is self-contained — repo, branch, design decisions, conventions, file map,
> build/test commands, and the prioritized to-do are all below. Verify line/symbol references
> against the live code before acting (the code is the source of truth; this is a point-in-time map).

## Environment

- **Repo:** `gluwa/creditcoin3` (private). Clone it; this is a large Rust + Substrate workspace.
- **Branch:** `attestor_v2_writability` (note: no middle "e"). Already pushed to `origin`.
- **Latest commits on the branch (most recent last):**
  - `7dd190456` `feat(attestor): USC write-ability message attestation` — the whole pipeline + shared crate + CLI + tests.
  - `fc581fb53` `chore(usc-messaging): remove superseded TS attester + relayer`.
  - `8e323b265` `chore(attestor): rename --message-attestation flag to --writeability`.
- **Toolchain:** the repo pins a Rust toolchain (`rust-toolchain.toml`). Foundry (`anvil`/`forge`/`solc`) is needed only for the `#[ignore]`d e2e test.

## What this feature is

Make the attestor a **message validator** for USC write-ability (cross-chain messaging,
Creditcoin L1 → other chains): watch the Creditcoin L1 **Outbox** for `MessagePublished` → sign the
canonical `messageHash` (ECDSA) → gossip a vote on `{chain_key}/message-votes/v1`. Relayers consume
the votes and deliver to a destination **Inbox**; the **attestor never relays**. Disabled by default
(opt-in via `--writeability`). Source-of-truth research lives in a sibling repo
`usc-write-ability-research` (esp. `documents/confluence-attestor-write-ability-poc.md`, the A1–A11
checklist, and `requirements/01-attesters-requirements.md`).

## Design decisions (preserve these — don't undo)

1. **Piggyback the existing attestor libp2p swarm.** Message votes are just a **new gossip topic**
   `{chain_key}/message-votes/v1` on the *same* swarm; discovery/peers/identify/kad unchanged. The
   p2p task (`tasks/p2p/mod.rs`) dispatches incoming frames by topic. (An earlier draft ran a
   separate swarm — rejected; do not reintroduce.)
2. **ECDSA / EOAValidator scheme** (not BLS) for message votes: sign the raw 32-byte `messageHash`,
   **no EIP-191 prefix**, 65-byte `(r,s,v)`. The EVM key is derived domain-separated from the
   attestor's existing seed (so one secret; the boot log prints the derived EVM address to register).
3. **One shared crate, re-exported.** `common/write-ability` holds `hash`, `envelope` (`MessageVote`
   SCALE wire type), `abi` (sol! bindings), `protocol` (topic + `chain_key_to_bytes32`). The
   `message-relayer` crate **re-exports** all four so producer (attestor) and consumer (relayer)
   cannot diverge on hash/wire/topic/ABI. Change these in ONE place.
4. **Config fallback for resolution.** Resolver prefers on-chain (chain-info precompile `0x…0fD3`
   → `IOutboxFactory.getOutbox`), but every input has a config override so the PoC runs before the
   factory/runtime fields exist. Static attester set works today; `OnChainValidator` does not yet.

## Project conventions (the user enforces these)

- **No co-author trailers** in commits (no `Co-Authored-By: Claude`/`Cursor`, no `Signed-off-by`).
  Plain title + body. (See `.cursor/rules/git-commits.mdc`.)
- **Run `cargo fmt --all` immediately before every commit** and amend if it changes anything — a
  file built/edited outside the normal per-crate sweep is the classic miss.
- **On any `Cargo.toml` change:** also run `taplo format` + `cargo machete`.
- **Verification trio on every Rust edit:** build/check, `cargo clippy --all-targets`, `cargo fmt --all`.
- **Never commit directly to `usc-dev`** (the integration branch); land work on a feature branch.
  Bring `usc-dev` into a feature branch via **`git rebase`, never a merge commit** (back up first).
- **Don't auto-commit/push** — ask first unless told to. **Don't create doc files unless asked.**

## Status

Attestor half **complete + tested**: whole workspace builds (`cargo build --workspace`), clippy/fmt/
taplo/machete clean, all unit/integration tests pass, anvil e2e passes. Cross-chain go-live is
blocked on §3 (colleague-owned) + the ABI unification.

### 1. Done ✅
- [x] Shared crate `common/write-ability` (hash, envelope, abi, protocol); relayer re-exports it.
- [x] A1 Config (`tasks/write_ability/config.rs`), opt-in, disabled by default.
- [x] A2 Resolver — chain-info precompile → `getOutbox` → assert `Outbox.chainKey()`; config fallbacks.
- [x] A3 Listener — `MessagePublished` poll + confirmation-depth finality gate.
- [x] A5 Signing — EVM secp256k1 from seed; raw-hash sign; recovery matches relayer.
- [x] A6 Gossip — new topic on the existing swarm (`tasks/p2p/mod.rs` dispatch-by-topic).
- [x] A7 Aggregator — unique signers, 2N/3+1, dedup.
- [x] A11 Anti-abuse — hard validation (decode + recover + signer ∈ set), chain-first allowlist, cap+TTL+LRU.
- [x] A9 Metrics / A10 Wiring — `MessageVote` counter; spawned in `lib.rs` JoinSet via `Shared.message_votes`.
- [x] CLI/config — `--writeability`, `--cc3-eth-url`, `ATTESTOR_*` env, `write_ability:` block in `attestor/config.yaml`.
- [x] Tests — hash vectors, signing, aggregator, ingest abuse, anvil e2e (`tests/e2e_anvil.rs`, `#[ignore]`).
- [x] Removed superseded TS attester + relayer from `usc-messaging/`.
- [x] Fixed pre-existing `message-relayer` build breakage + clippy warnings.

### 2. Attestor follow-ups ⏳ (same owner — hardening)
- [ ] `AttesterSet::OnChainValidator` — only `Static` works today (reading `IVoteValidator.attesters()` needs a destination-chain RPC; bails with a clear log).
- [ ] Probabilistic finality when the gadget is paused (research §6.8) — fixed confirmation-depth today.
- [ ] GossipSub peer-scoring + per-peer rate limits for the message-vote topic (defaults today).
- [ ] `getOutbox` → `address(0)` backoff + subscribe `OutboxCreated` (today: fail-fast).
- [ ] Publish-before-mesh — gate/buffer publish until the topic has mesh peers (today best-effort; early publish dropped).
- [ ] Vote persistence across restart (in-memory by design for PoC).

### 3. Colleague-owned blockers 🚧 (required for live cross-chain)
- [ ] **Runtime `SupportedChain` write-ability fields + migration + RPC** (R1–R3): `write_ability_chain_key` (bytes32) + `message_attestation_enabled`, via `get_supported_chain`. **Conditional importance:** only required if the bytes32 chain key is registry-assigned/opaque; if it's the right-padded-`u64` convention (research §2.3 option A, what `chain_key_to_bytes32` implements) the attestor can derive it and this is ops-only. `outbox_factory_address` already shipped (PR #873, already merged on this branch).
- [ ] **`OutboxFactory` contract** (`getOutbox(bytes32)`) + factory-created Outbox. Use `outbox_address` config override until then.
- [ ] **Production `EOAValidator`** replacing PoC `DummyVoteValidator` (accepts everything) + a process to keep its on-chain attester address set in sync with attestors' derived EVM addresses.
- [ ] **⚠️ ABI unification (the #1 PoC blocker).** Three shapes disagree:
  - Binding (`common/write-ability/src/abi.rs`): `MessagePublished(bytes32 messageId, address emitterAddress, bytes payload)` + expects `Outbox.chainKey()`.
  - PoC `usc-messaging/contracts/src/SimpleOutbox.sol`: `MessagePublished(bytes32 messageId, bytes32 emitterAddress, bool requiresAck, bytes payload)` and has **no `chainKey()`**.
  - Research spec: `MessagePublished(bytes32 messageId, address emitterAddress, bool requiresAck, bytes payload)`.
  Until reconciled, the listener's event-signature filter won't match real Outbox logs and nothing decodes. Recommended: align the binding to the spec (add `requiresAck`, keep `address`) and add `chainKey()` to the contract.

### 4. End-to-end / infra 🧪
- [ ] Full zombienet binary e2e — N real attestors + CC3 node + Outbox, votes over real gossip → quorum → relayer delivers. (Anvil e2e covers EVM→hash→sign→ingest→quorum but skips the gossip hop + full attestor startup.)
- [ ] Relayer↔attestor live interop — relayer's `tests/e2e_anvil.rs` is still `unimplemented!()`.
- [ ] CI lane with foundry to run the `#[ignore]`d anvil e2e.
- [ ] Golden vector vs the deployed `validateVotes` once the real validator exists.
- [ ] Operator runbook (env, topic, boot-logged signer address to register) — research D2.

## messageHash pinning (deploy-time contract, not code)

All three parties must compute the identical `keccak256(abi.encode(bytes32 messageId, address
emitterAddress, bytes32 destinationChainKey, uint256 creditcoinChainId, bytes payload))`. Rust side
is fixed via the shared crate. The deployed contracts must agree:
- `Outbox.chainKey()` (CC3 L1) **==** the Inbox's configured destination key **==** what the attestor binds.
- The Inbox's baked `creditcoinChainId` constant **==** the CC3 EVM `eth_chainId` (dev `42`, testnet `102036`).
- `creditcoinChainId` must be encoded as **uint256**; use `abi.encode` (not `encodePacked`); sign the raw hash (no EIP-191). Any mismatch → `ecrecover` yields a wrong signer → `validateVotes` reverts → silent total delivery failure.

## Most important changes to get the PoC working (priority order)

1. **Unify the Outbox `MessagePublished` ABI** (§3) — the one true blocker; one small change unblocks the flow.
2. **Pin the messageHash inputs** across the deployed Outbox/Inbox (above).
3. Config-only: `outbox_address` override (skip factory), `Static` attester set, keep `DummyVoteValidator`. No `OutboxFactory`/`EOAValidator`/runtime fields needed for the PoC.
The quoter is a **separate** economic-layer workstream (relayer team), not needed for the PoC.

## How to enable

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
CLI equivalent: `--writeability --cc3-eth-url ws://…` (env: `ATTESTOR_WRITEABILITY`, `ATTESTOR_CC3_ETH_URL`).
On boot the attestor logs its **derived EVM signer address** — register it in the `EOAValidator` and list it in `attesters`.

## Build & test

```bash
cargo build --workspace                                       # whole project must compile
cargo test -p write-ability -p attestor -p message-relayer    # unit + integration
cargo clippy --all-targets && cargo fmt --all                 # before any commit
cargo test -p attestor --test e2e_anvil -- --ignored          # anvil e2e (needs foundry on PATH)
```
**Gotcha:** if a build hangs at 0% CPU, it's blocked on the cargo `~/.cargo/.package-cache` lock held
by rust-analyzer's background `cargo check --workspace`. Either let RA finish, or build with a
separate `CARGO_TARGET_DIR=/tmp/<dir>` to sidestep it.

## Key files

| Path | What |
|------|------|
| `common/write-ability/` | shared hash / envelope / abi / protocol (relayer re-exports) |
| `attestor/attestor/src/tasks/write_ability/` | config, resolver, listener, signing, aggregator, ingest, mod |
| `attestor/attestor/src/tasks/p2p/mod.rs` | topic subscribe + dispatch + publish (piggyback) |
| `attestor/attestor/src/shared.rs` | `message_votes: Option<Arc<MessageVoteState>>` |
| `attestor/attestor/src/main.rs` | CLI/config (`build_write_ability`, `--writeability`) |
| `attestor/attestor/tests/e2e_anvil.rs` | anvil end-to-end (`#[ignore]`) |
| `attestor/config.yaml` | documented `write_ability:` section |
| `message-relayer/` | the Rust relayer (consumes votes, delivers to Inbox); PoC-grade |
| `usc-messaging/contracts/` | PoC Solidity (Outbox/Inbox/DummyVoteValidator/dApp) |
| `usc-messaging/src/quoter/`, `src/dApp-ack-worker/` | remaining TS components (quoter is a separate workstream) |
