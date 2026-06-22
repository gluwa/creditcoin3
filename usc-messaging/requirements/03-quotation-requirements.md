# Quotation System — Requirements & Implementation Plan

## Document Purpose

This document translates the usc-write-ability-research into specific requirements for the Quotation System. Implementation has started in `usc-messaging/src/quoter`. Quotation should be built **before or in parallel with** relayers, since the relayer contract accepts a quote. Each relayer network typically builds its own quotation system and does not trust another network's quotes.

## Executive Summary

The quotation system provides fee quotes for cross-chain messaging. It must know exchange rates between the destination chain's native currency and attest coin and/or the relayer payment token. A **core fee** (flat fee in attest coin) may be introduced. For the first release, keep it simple—likely attest coin only. The system is built per relayer network; we need it first because we will be the only relayer network initially.

---

## Scope

### In Scope

1. **Exchange Rate Knowledge**
   - Native currency of destination chain ↔ attest coin
   - Native currency of destination chain ↔ relayer payment token (if different from attest coin)
   - Used to convert gas costs and buffers to payment token

2. **Quote Structure**
   - Relayer contract accepts a signed quote
   - Quote includes: relay price, acknowledgment price (if `requiresAck`), payee address, payment token, expiry, signature
   - Quoter signs quotes; relayer contract validates signature against whitelist

3. **Core Fee (Optional)**
   - Flat fee in attest coin
   - May be introduced as a protocol-level fee

4. **First Release Simplification**
   - Likely attest coin only for payment
   - Other tokens (e.g., wrapped CTC, ERC20) may be deferred
   - Keeps implementation and testing manageable

5. **Per Relayer Network**
   - Each relayer network builds its own quotation system
   - Relayer networks typically do not trust another network's quotes
   - We build ours because we provide the first relayer network

### Out of Scope (for first release)

- Multiple payment tokens (consider attest coin only)
- Gas topping / IGP-style mechanism (can be added later)
- Cross-relayer-network quote aggregation

---

## Key Constraints (from CTO)

| Constraint | Description |
|------------|-------------|
| **Build before relayers** | Quotation needed before or in parallel with relayers; relayer contract accepts quote. |
| **Exchange rates required** | Must know: native currency of other chain ↔ attest coin and/or relayer payment token. |
| **Core fee** | May introduce flat fee in attest coin. |
| **First release simple** | Probably attest coin only; other tokens later. |
| **Per relayer network** | Each network builds its own; typically won't trust another's quoting. |
| **We build first** | We are the only relayer network at first, so we need our quotation system. |

---

## Architecture

### Data Flow

```
dApp → Request quote (off-chain API)
         ↓
Quoter Service → Fetches: gas prices, exchange rates
         ↓
Quoter Service → Computes: relayPrice, ackPrice (if requiresAck)
         ↓
Quoter Service → Signs quote with EOA
         ↓
dApp → Calls RelayerContract.validateAndCollectFee(signedQuote)
dApp → Calls Outbox.publishMessage(requiresAck, payload)
```

### Exchange Rates Needed

| Rate | Purpose |
|------|---------|
| Destination chain native → attest coin | Convert gas cost to payment token |
| Destination chain native → relayer payment token | If payment token ≠ attest coin |
| USD buffer → attest coin | Overhead buffer for volatility |

### Quote Calculation (Conceptual)

1. Estimate destination chain gas for `deliverMessage` + target contract execution.
2. Add overhead buffer (USD-denominated, converted to payment token).
3. If `requiresAck`: add cost for proof submission on Creditcoin L1 (~500k gas).
4. Convert total to payment token using current exchange rates.
5. Optionally add core fee (flat attest coin).

---

## Development Approach: Dummy Contracts & Dummy Data

### Rationale

To unblock development before real exchange rate feeds and production quoter:

1. Use **dummy quoter service** that returns fixed quotes.
2. Use **fixed exchange rates** (e.g., 1:1 or configurable constants).
3. Use **dummy relayer contract** that accepts any signature from a dev key.
4. Validate end-to-end flow: quote → payment → publish.

### Dummy Quoter Service

- REST or RPC API: `GET /quote?destinationChain=X&requiresAck=true`
- Returns: `{ relayPrice, ackPrice, payeeAddress, paymentToken, expiry, signature }`
- Signature: sign with dev EOA; relayer contract whitelists this address
- Exchange rates: hardcoded (e.g., 1 ETH = 1000 attest coin for testing)

### Dummy Exchange Rate Source

| Component | Dummy Approach |
|-----------|----------------|
| **Gas price** | Use public RPC `eth_gasPrice` or fixed value |
| **Exchange rate** | Config file: `{ "ETH": 1000, "BNB": 500 }` (units per attest coin) |
| **Quoter** | Single dev EOA; whitelist in relayer contract |
| **Quote expiry** | 1 hour or 24 hours for testing |

### Development Flow

1. **Phase 1**: Dummy quoter API returns fixed quote; dummy relayer contract accepts it.
2. **Phase 2**: Integrate real gas price from destination chain RPC.
3. **Phase 3**: Integrate exchange rate API (e.g., Chainlink, custom oracle, or config).
4. **Phase 4**: Add core fee (flat attest coin) if required.
5. **Phase 5**: Production quoter with proper key management and rate limits.

### TODOs (Real Pricing)

- [ ] **Exchange rates**: Replace hardcoded 1:1 with real source. Options: Chainlink price feeds (on-chain), DEX spot (Uniswap reserves), off-chain API (CoinGecko, etc.), or config file with manual updates.
- [ ] **Gas estimation**: Replace fixed gas limit with dynamic estimation for `deliverMessage` + target contract execution (e.g. per-payload heuristic or simulation).
- [ ] **Overhead buffer**: Define USD-denominated buffer size per chain (volatility-based); convert to payment token.
- [ ] **Core fee**: Design and implement flat attest coin fee if required.

---

## Deliverables

| # | Deliverable | Owner |
|---|-------------|-------|
| 1 | Dummy quoter API (fixed quotes, dev signature) | Protocol / Backend |
| 2 | Exchange rate module (config or API) | Protocol Team |
| 3 | Gas estimation for destination chain delivery | Protocol Team |
| 4 | Relayer contract: quote validation (signature, expiry, token) | Smart Contract Team |
| 5 | Core fee design (if applicable) | Research / Protocol |
| 6 | Production quoter service with key management | Protocol Team |

---

## Dependencies

- **Relayer contract** — Must have `validateAndCollectFee(signedQuote)` and quoter whitelist.
- **Outbox** — dApp calls outbox after relayer; no direct dependency.
- **Exchange rate source** — Needed for accurate quotes; dummy acceptable for dev.

---

## Open Questions

1. **Core fee**: Exact amount and collection mechanism (to protocol treasury vs relayer pool)?
2. **Exchange rate source**: Chainlink, custom oracle, or off-chain API?
3. **Quote expiry**: Recommended window (e.g., 5 min, 1 hour)?
4. **Overhead buffer**: How to size buffer per chain (volatility-based)?
5. **Multi-token**: When to support ERC20 payment tokens beyond attest coin?

---

## Related Documents

- [Quotation System](../07-quotation-system.md)
- [Relayers Requirements](./02-relayers-requirements.md)
- [Architecture Overview](../01-architecture-overview.md)
