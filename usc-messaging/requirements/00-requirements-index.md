# USC Write-Ability Layer — Requirements Index

## Overview

This folder contains implementation plans and requirements derived from the usc-write-ability-research. Each document defines scope, constraints, and a development approach using **dummy contracts and dummy data** to unblock the protocol team before full deployment.

## Documents

| Document | Description |
|----------|-------------|
| [01-attesters-requirements.md](./01-attesters-requirements.md) | Attesters (message validators): vote on outbox events, P2P only, no relaying |
| [02-relayers-requirements.md](./02-relayers-requirements.md) | Relayers: pick up messages, deliver to inbox, one contract per chain, many clients |
| [03-quotation-requirements.md](./03-quotation-requirements.md) | Quotation: exchange rates, core fee, build before relayers |

## Key Principles (from CTO)

1. **Attester ≠ Relayer** — Attester clients must never be relayers. Mixing validation with delivery is forbidden.
2. **Separation of concerns** — Attesters scale as few; relayers scale as many. Design for this.
3. **One Relayer Contract per client chain** — Many Relayer Clients can share across chains.
4. **Quotation before relayers** — Relayer contract accepts a quote; build quotation in parallel or first.
5. **Per relayer network** — Each relayer network builds its own quotation; typically won't trust another's.

## Development Strategy: Dummy Contracts

All plans recommend using dummy contracts and dummy data to start development immediately:

| Component | Dummy Approach |
|-----------|----------------|
| **Outbox** | Minimal contract emitting `MessagePublished` on demand |
| **Relayer Contract** | Accepts dummy quotes, minimal validation |
| **Inbox** | Dummy vote validator (accepts dev signatures) |
| **Quoter** | Fixed quotes, dev EOA signature, config-based exchange rates |
| **P2P** | Mock votes or pre-signed 2/3+1 threshold |

## Build Order (Suggested)

1. **Quotation** — Dummy quoter + exchange rate module (needed by relayer contract)
2. **Attesters** — Dummy outbox → event listener → P2P vote submission
3. **Relayers** — Dummy inbox + relayer client → deliver with pre-signed votes
4. **Integration** — Swap dummies for production contracts as they become available

## Research Updates Required

The following statements in existing research docs must be corrected:

- **01-architecture-overview.md**: Remove "Relayers can be run by attesters" and "attesters can participate as relayers"
- **07-quotation-system.md**: Same removals; QoS should not rely on attester-relayer overlap

## Related Research

https://github.com/gluwa/usc-write-ability-research
