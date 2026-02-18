# Relayers — Requirements & Implementation Plan

## Document Purpose

This document translates the usc-write-ability-research into specific requirements for the Relayer component of the USC Write-Ability Layer. It defines scope, architecture (one-to-many: one relayer contract, many relayer clients), and a development approach using dummy contracts and dummy data.

## Executive Summary

Relayers pick up messages and **deliver** them. They need to hold native currency on the destination chain. They are **not** part of security—they are paid for delivery. Attester clients must **never** be relayers. The model is one Relayer Contract per client chain, with many Relayer Clients (one-to-many) that can be shared across chains.

---

## Scope

### In Scope

1. **Relayer Contract** — On-chain contract per client chain that:
   - Represents a relayer network
   - Accepts quotes (from quotation system)
   - Handles fee collection and payment routing
   - Does **not** call the outbox contract (core protocol independence)
   - Distributes profits to relayer clients (implementation detail of the network)

2. **Relayer Clients** — Off-chain software that:
   - Listens to the relayer contract (and/or P2P votes for message readiness)
   - Picks up messages that have reached 2/3+1 attestation threshold
   - Delivers messages to the inbox contract on the destination chain
   - Holds native currency of the destination chain (for gas)
   - Can be **shared across all chains** (one relayer client can service multiple relayer contracts)

3. **One-to-Many Model**
   - One Relayer Contract per client chain
   - Many Relayer Clients per Relayer Contract
   - Relayer clients can service multiple relayer contracts (multi-chain)
   - Profit distribution is internal to the relayer network

4. **Trust Model**
   - Relayers require **no trust** for security
   - They are paid to service delivery
   - Core protocol guarantees attestation; relayers only execute delivery

### Out of Scope

- Attestation / voting (handled by attesters)
- Quotation logic (handled by quotation system; relayer contract accepts quotes)
- Running attester and relayer in the same client (forbidden)

---

## Key Constraints (from CTO)

| Constraint | Description |
|------------|-------------|
| **Attester ≠ Relayer** | An attester client must **never** be a relayer. Do not consider it. |
| **No trust for security** | Relayers are paid for delivery; they are not part of the security model. |
| **Native currency** | Relayers need to hold a lot of native currency on the destination chain. |
| **One contract per chain** | One Relayer Contract per client chain. |
| **Many clients** | Many Relayer Clients can represent one Relayer Contract; profits distributed later. |
| **One-to-many** | One Relayer Contract → many Relayer Clients. Other networks may have 100s of decentralized relayers. |
| **Share clients** | Relayer clients can be shared for all chains. |
| **Quotation first** | Relayer contract accepts a quote; quotation system should be built before or in parallel. |

---

## Architecture

### Contract Model

```
Relayer Contract (per client chain)
    ├── Accepts signed quotes
    ├── Validates quote (signature, expiry, payment token)
    ├── Collects fees → payeeAddress (relayer pool)
    └── Does NOT call outbox

Relayer Clients (many, can be shared across chains)
    ├── Listen to P2P for messages with 2/3+1 votes
    ├── Listen to relayer contract for payment/quote context (if needed)
    ├── Deliver to inbox on destination chain
    └── Hold native currency of destination chain
```

### Message Flow

1. dApp calls RelayerContract (fee validation/payment) then Outbox (publish) — two separate calls.
2. Attesters vote on P2P.
3. Relayer clients observe votes; when 2/3+1 reached, pick up message.
4. Relayer client delivers to inbox on destination chain (pays gas in native currency).
5. Relayer pool distributes profits to clients based on performance (network-specific).

### What Relayer Clients Listen To

- **P2P layer**: Messages that have reached 2/3+1 attestation threshold
- **Relayer contract**: For quote/payment context (optional; depends on design)
- **Outbox**: Not directly—messages are validated via P2P votes

---

## Conflicts with Existing Research

The following must be **removed or corrected**:

- **"Relayers can be run by attesters"** — **Incorrect.** Forbidden.
- **"Attesters can participate as relayers"** — **Incorrect.** Forbidden.
- **QoS via attester-relayer overlap** — **Incorrect.** Use separate mechanisms (e.g., spy nodes, relayer network incentives).

---

## Development Approach: Dummy Contracts & Dummy Data

### Rationale

To unblock development before full quotation and relayer contract deployment:

1. Use **dummy relayer contracts** that accept placeholder quotes.
2. Use **dummy inbox contracts** on a test chain for delivery.
3. Use **mock P2P** or **pre-signed votes** to simulate 2/3+1 threshold.
4. Use **testnet native currency** for gas.

### Dummy Relayer Contract

Deploy a minimal contract that:

- Has `validateAndCollectFee(bytes calldata signedQuote)` that always succeeds (or validates a dummy signature).
- Transfers payment to a configurable `payeeAddress`.
- Does not integrate with a real quoter initially.

**Suggested interface**:

```solidity
interface IDummyRelayerContract {
    function validateAndCollectFee(bytes calldata signedQuote) external payable;
    // Quote structure can be minimal: payeeAddress, amount, expiry, signature
}
```

### Dummy Inbox Contract

Deploy a minimal inbox that:

- Accepts `deliverMessage(messageId, emitterAddress, payload, votes)`.
- Validates votes against a **dummy validator** (e.g., accepts any signature from a whitelisted dev key).
- Calls `receiveMessage` on a test destination contract.
- Emits `MessageDelivered`.

### Dummy Data Strategy

| Component | Dummy Approach |
|-----------|----------------|
| **Relayer contract** | Deploy to Hardhat/Anvil; dummy signature validation |
| **Inbox** | Deploy to same or second test chain; dummy vote validator |
| **P2P votes** | Pre-generate 2/3+1 signatures from dev keys; or mock gossip |
| **Quote** | Fixed structure: payeeAddress, amount=0 or small, expiry far future |
| **Native currency** | Use testnet ETH/BNB/etc. from faucets |

### Development Flow

1. **Phase 1**: Relayer client listens to mock P2P (or file/queue) for "ready" messages.
2. **Phase 2**: Relayer client calls dummy inbox `deliverMessage` with pre-signed votes.
3. **Phase 3**: Integrate with dummy relayer contract for fee flow (optional for initial dev).
4. **Phase 4**: Integrate with real quotation system and relayer contract.
5. **Phase 5**: Swap dummy inbox/vote validator for production.

---

## Deliverables

| # | Deliverable | Owner |
|---|-------------|-------|
| 1 | Dummy relayer contract (accepts dummy quotes) | Smart Contract Team |
| 2 | Dummy inbox + dummy vote validator for testing | Smart Contract Team |
| 3 | Relayer client: listen to P2P for 2/3+1 messages | Protocol Team |
| 4 | Relayer client: deliver to inbox, pay gas in native currency | Protocol Team |
| 5 | Relayer pool / profit distribution design (optional for v1) | Protocol / Smart Contract |
| 6 | Update architecture docs to remove attester-relayer overlap | Research |

---

## Dependencies

- **Quotation system** — Relayer contract accepts quotes; build quotation before or in parallel.
- **Attesters** — Must produce votes so relayer clients know when to deliver.
- **Inbox contract** — Must be deployed on destination chain.
- **Outbox contract** — For message publishing (dApp flow); relayer does not call it.

---

## Open Questions

1. **Relayer registration**: Does the relayer contract track which clients are part of the network?
2. **Profit distribution**: On-chain vs off-chain; criteria (messages delivered, latency, etc.)?
3. **Gas topping**: Support for additional gas if initial quote insufficient (Hyperlane IGP-style)?
4. **Multi-relayer networks**: How do different relayer networks compete? Same outbox, different relayer contracts?

---

## Related Documents

- [Architecture Overview](../01-architecture-overview.md)
- [Quotation System](../07-quotation-system.md)
- [Quotation Requirements](./03-quotation-requirements.md)
- [Inbox Contract](../04-inbox-contract.md)
- [Attesters Requirements](./01-attesters-requirements.md)
