# Attesters (Message Validators) — Requirements & Implementation Plan

## Document Purpose

This document translates the usc-write-ability-research into specific requirements for the Attester component of the USC Write-Ability Layer. It defines scope, constraints, and a development approach using dummy contracts and dummy data.

## Executive Summary

Attesters are **message validators** that pick up messages from outbox votes, vote on the P2P layer, and focus exclusively on security. They do **not** relay messages. Mixing validation with delivery is explicitly forbidden—attesters and relayers scale differently (few attesters, many relayer clients).

---

## Scope

### In Scope

1. **Attester Client** — Software that:
   - Listens to one outbox contract per chain (one contract, one event hash per chain)
   - Picks up `MessagePublished` events from the outbox
   - Votes on the P2P layer for message validation
   - Votes on **finalized** events, or on **probabilistic finality** when finality is paused (with a bounded lag—cannot fall too far back)
   - Does **not** require funds to operate
   - Does **not** relay or deliver messages

2. **Per-Chain Attester Model**
   - One attester instance per client chain
   - Cannot share a single attester across many chains
   - Each attester listens to one outbox contract (one chain)

3. **Finality Rules**
   - Primary: vote on finalized events
   - Fallback: if finality is paused, vote on probabilistic finality
   - Constraint: cannot fall too far behind (lag must be bounded)

4. **Integration with Existing USC**
   - Extends the existing P2P voting layer (attestor-gossip)
   - Uses existing attestation infrastructure where applicable
   - Archive nodes for Creditcoin L1 and client chains (for reading state)

### Out of Scope

- Message delivery / relaying (handled by relayers)
- Quotation or fee logic (handled by relayer network)
- Acknowledgment voting (delivery uses native USC proving directly)
- Running attester and relayer in the same client (forbidden)

---

## Key Constraints (from CTO)

| Constraint | Description |
|------------|-------------|
| **No relaying** | Attester clients must **never** act as relayers. Mixing validation with delivery is a design mistake. |
| **No funds** | Attesters do not need native currency or tokens to operate. |
| **Per-chain** | One attester per chain. Cannot share attester across many chains. |
| **One contract, one event** | For listening: one outbox contract, one event hash per chain. Keep it simple. |
| **Finality** | Vote on finalized events; if finality paused, use probabilistic finality with bounded lag. |
| **Scaling** | Few attesters, many relayer clients. Design for this asymmetry. |

---

## Architecture

### Listening Model

```
Outbox Contract (per chain) → MessagePublished event
         ↓
Attester Client (per chain) → Parse event, compute message hash
         ↓
P2P Voting Layer → Sign and gossip vote
         ↓
2/3+1 threshold → Message ready for relay (by relayers, not attesters)
```

### Message Hash (for voting)

Attesters sign the hash:

```
keccak256(abi.encode(messageId, emitterAddress, destinationChainKey, creditcoinChainId, payload))
```

- `destinationChainKey` — from outbox (chain-agnostic)
- `creditcoinChainId` — **Local substrate node chain ID** (from `pallet-evm-chain-id` in the Creditcoin runtime). This is the chain identifier exposed when connecting to the Creditcoin node via EVM RPC (`eth_chainId`).

**Values from creditcoin3-next** (see `node/src/chain_spec.rs`):

| Environment | creditcoinChainId | Source |
|-------------|-------------------|--------|
| Development | `42` | `SS58Prefix::get() as u64` |
| Testnet | `102036` | `EVM_CHAINID` constant |
| Devnet | From chainspec | `chainspecs/devnetSpecRaw.json` |

### Event to Listen For

- **Contract**: One outbox contract per client chain
- **Event**: `MessagePublished(bytes32 messageId, address emitterAddress, bool requiresAck, bytes payload)`
- **Event hash**: Fixed for the outbox ABI

---

## Integration with Existing Attestor Network

The attestor network lives in `creditcoin3-next/attestor`. This section gives rough guidelines for integrating message attestation into that codebase.

### Current Attestor Architecture

| Component | Location | Role |
|-----------|----------|------|
| **Attestor binary** | `attestor/attestor/src/main.rs` | Entry point; config, workers, CC3/ETH clients |
| **Chain listeners** | `attestor/attestor/src/chain_listener/` | ETH (source chain blocks), CC3 (attestation state) |
| **Worker threads** | `attestor/attestor/src/worker/` | Production, validation, P2P, API |
| **P2P** | `attestor/attestor/src/worker/p2p/` | Gossipsub, topics per `chain_key` |

**Flow today**: ETH blocks → production worker → attestation → P2P gossip → validation pool → quorum → submit inherent to CC3.

**Message attestation flow**: Outbox events → message listener → message vote → P2P gossip → message validation pool → quorum → relayers pick up (no inherent).

### Integration Options

1. **New chain listener for outbox**
   - Add `chain_listener/outbox/` (or similar) that subscribes to `MessagePublished` on Creditcoin via EVM RPC (`cc3_url` or a dedicated EVM endpoint).
   - One outbox listener per `chain_key` (same per-chain model as the attestor).

2. **Reuse or extend P2P**
   - Gossipsub topics: `{chain_key}/attest` for block attestations; add `{chain_key}/message` (or similar) for message votes.
   - Separate gossip topic keeps message votes separate from block attestations.
   - Reuse existing P2P swarm, identify, and gossip infrastructure.

3. **New worker or extend production**
   - Option A: New `WorkerMessageAttestation` that listens to outbox events and produces message votes.
   - Option B: Extend production worker with an optional outbox listener that feeds into a separate vote channel.
   - Message votes are independent of block attestations; no inherent submission.

4. **Message validation pool**
   - Message votes need a quorum (2/3+1) similar to block attestations.
   - Add a message-specific validation pool or extend the existing pool with a different message type.
   - Different signature scheme: message votes use EOA/ECDSA (or TSS/BLS per research) over the message hash; block attestations use BLS.

5. **Config**
   - Add `outbox_address` (or `outbox_addresses` per chain) to attestor config.
   - Optional: `--message-attestation` flag to enable/disable message attestation.

### Key Differences

| Aspect | Block attestation | Message attestation |
|--------|-------------------|---------------------|
| Source | ETH RPC (source chain blocks) | CC3 EVM (outbox on Creditcoin) |
| Output | Inherent to CC3 | P2P gossip only (relayers consume) |
| Signature | BLS (attestor pool) | EOA/ECDSA or TSS (per research) |
| Topic | `{chain_key}/attest` | `{chain_key}/message` (or similar) |

### Suggested Implementation Order

1. Add outbox chain listener (subscribe to `MessagePublished` via CC3 EVM logs).
2. Define message vote structure and serialization.
3. Add message gossip topic and P2P handling for message votes.
4. Add message validation pool with quorum logic.
5. Wire outbox listener → P2P → validation pool in main loop.

---

## Conflicts with Existing Research

The following in `01-architecture-overview.md` and `07-quotation-system.md` must be **removed or corrected**:

- **"Relayers can be run by attesters"** — **Incorrect.** Attester client must never be a relayer.
- **"Attesters can participate as relayers"** — **Incorrect.** Same as above.
- **QoS via attester-relayer overlap** — **Incorrect.** Security (attestation) and delivery (relaying) are separate roles.

---

## Development Approach: Dummy Contracts & Dummy Data

### Rationale

To unblock development before full outbox/inbox deployment:

1. Use **dummy outbox contracts** that emit `MessagePublished` with synthetic data.
2. Use **dummy event hashes** and **dummy chain keys** for testing.
3. Feed attester clients with **dummy RPC endpoints** or **mock event streams**.

### Dummy Outbox Contract

Deploy a minimal contract that:

- Emits `MessagePublished(messageId, emitterAddress, requiresAck, payload)` on demand (e.g., via `publishTestMessage()`).
- Uses deterministic or configurable `chainKey` / `destinationChainKey`.
- Does not require factory, validator, or acknowledgment logic.

**Suggested interface**:

```solidity
interface IDummyOutbox {
    function publishTestMessage(
        bytes32 messageId,
        address emitterAddress,
        bool requiresAck,
        bytes calldata payload
    ) external;
}
```

### Dummy Data Strategy

| Component | Dummy Approach |
|-----------|----------------|
| **Outbox address** | Deploy to local Hardhat/Anvil; use fixed address in config |
| **ChainKey** | Use `bytes32(0x01)` or similar for dev |
| **creditcoinChainId** | Use `42` (development) or `102036` (testnet) — see `node/src/chain_spec.rs` |
| **Message payload** | Fixed `abi.encode(destinationContract, payloadData)` |
| **Event stream** | Script or cron that calls `publishTestMessage` periodically |

### Development Flow

1. **Phase 1**: Attester client connects to dummy outbox RPC, subscribes to `MessagePublished`.
2. **Phase 2**: On each event, compute message hash and submit vote to P2P (or mock P2P).
3. **Phase 3**: Integrate with real attestor-gossip when ready.
4. **Phase 4**: Swap dummy outbox for real outbox once deployed.

---

## Deliverables

| # | Deliverable | Owner |
|---|-------------|-------|
| 1 | Dummy outbox contract (Solidity) emitting `MessagePublished` | Smart Contract Team |
| 2 | Attester client: event listener + message hash computation | Protocol Team |
| 3 | Attester client: P2P vote submission (extend attestor-gossip) | Protocol Team |
| 4 | Finality logic: finalized vs probabilistic with bounded lag | Protocol Team |
| 5 | Per-chain attester configuration and deployment docs | Protocol Team |
| 6 | Update `01-architecture-overview.md` to remove attester-relayer overlap | Research |

---

## Dependencies

- Outbox contract interface (or dummy) must be stable.
- P2P voting layer (attestor-gossip) must support message validation votes.
- Archive node access for Creditcoin L1 and client chains.

---

## Open Questions

1. **Probabilistic finality**: Exact definition of "cannot fall too far back" (block depth, time window)?
2. **Attestor-gossip**: Does the existing gossip protocol need a new message type for outbox votes?
3. **Chain registry**: How does attester discover outbox address per `chainKey`?

---

## Related Documents

- [Architecture Overview](../01-architecture-overview.md)
- [Outbox Contract](../03-outbox-contract.md)
- [Message Protocol](../02-message-protocol.md)
- [Relayers Requirements](./02-relayers-requirements.md)
- `creditcoin3-next/attestation_doc.md` — existing attestor architecture