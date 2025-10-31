# Creditcoin3 Architecture Documentation

This directory contains comprehensive architecture documentation for Creditcoin3's query proof system.

## Quick Start

**New to the system?** Start here:
1. [WHAT_IS_BEING_PROVEN.md](./WHAT_IS_BEING_PROVEN.md) - Understanding what proofs actually prove
2. [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md) - How proofs are generated from start to finish
3. [BLOCK_DATA_FLOW_DIAGRAMS.md](./BLOCK_DATA_FLOW_DIAGRAMS.md) - Visual diagrams of data flow

**Want to implement automatic event handling?**
- [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) - Complete implementation guide

**Deciding between STARK and native precompiles?**
- [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Detailed comparison

---

## Document Overview

### Core Concepts

#### [WHAT_IS_BEING_PROVEN.md](./WHAT_IS_BEING_PROVEN.md)
**What it covers:**
- The complete picture of query proofs
- Merkle proof of inclusion
- Continuity chain (the key innovation)
- Security properties and guarantees
- Real-world examples

**Read this if you want to understand:**
- What exactly is being proven?
- How does the system ensure data integrity?
- What are the security guarantees?

---

### Implementation Details

#### [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md)
**What it covers:**
- Complete end-to-end proof generation flow
- How block data is sourced from external chains
- Step-by-step process from query to verification
- Native precompile architecture
- Automatic query generation patterns

**Read this if you want to understand:**
- How does the prover service work?
- Where does block data come from?
- How can contracts automatically generate queries?
- What's the role of off-chain components?

**Key insight:** Block data MUST come from off-chain. The native precompile doesn't eliminate this requirement - it only makes verification faster.

#### [BLOCK_DATA_FLOW_DIAGRAMS.md](./BLOCK_DATA_FLOW_DIAGRAMS.md)
**What it covers:**
- Visual diagrams showing data flow
- Comparison of STARK vs native precompile flows
- Event-driven automatic query patterns
- Data sources and dependencies

**Read this if you want to understand:**
- Visual representation of the system
- How data flows from source chain to verification
- Timeline comparisons between approaches
- Where each piece of data comes from

---

### Decision-Making Guides

#### [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md)
**What it covers:**
- Real-world performance data (15-minute STARK proof generation)
- Cost comparison (STARK: $20K+/month, Native: $1K/month)
- Trust model differences
- Cross-chain implications
- Decision matrix for different use cases

**Read this if you need to decide:**
- STARK or native precompile for my application?
- Can I accept 15-minute proof generation?
- What are the cost implications?
- Do I need trustless cross-chain verification?

**Key findings:**
- STARK: 15 minutes, trustless, expensive, universal
- Native: <100ms, fast, cheap, requires trust in CC3 validators

---

### Practical Guides

#### [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md)
**What it covers:**
- Problem statement: How to automatically verify external events
- Architecture options (centralized relayer, decentralized oracles, user-submitted, hybrid)
- Complete implementation guide with code examples
- Deployment instructions
- FAQ and troubleshooting

**Read this if you want to:**
- Implement automatic event handling
- Build a relayer service
- Understand different architecture patterns
- See working code examples

**Includes:**
- Full Node.js relayer implementation
- Solidity smart contract examples
- Merkle tree implementation
- Deployment scripts

---

### Technical Details

#### [QUERY_HASH_SIMPLIFICATION.md](./QUERY_HASH_SIMPLIFICATION.md)
**What it covers:**
- Query ID computation flow
- Proposed simplifications
- Implementation changes required
- Migration plan

**Read this if you're:**
- Working on the Cairo program
- Modifying query processing
- Investigating query hash issues

#### [WHY_FELTS_NOT_BYTES.md](./WHY_FELTS_NOT_BYTES.md)
**What it covers:**
- Why the system uses field elements (felts) instead of bytes
- Cairo constraints and limitations
- Design decisions and trade-offs

**Read this if you're:**
- Working with Cairo programs
- Understanding data encoding
- Debugging felt-related issues

---

## Common Questions Answered

### Q: If a dApp contract wants to automatically generate a query based on an event, how does it provide block data?

**A:** The contract itself cannot provide block data - blockchains cannot make HTTP requests to other chains. You need an **off-chain relayer service** that:

1. Monitors the source chain for events
2. Fetches block data via RPC when an event occurs
3. Builds the merkle tree and proof locally
4. Submits the proof to the target chain for verification

See [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) for complete implementation details.

### Q: Does the native precompile eliminate the need for off-chain components?

**A:** No. The native precompile only makes **verification** faster (<100ms vs 15 minutes). You still need off-chain components to:
- Fetch block data from source chains
- Build merkle trees
- Generate proofs
- Submit to target chain

The precompile just verifies the data is correct - it doesn't fetch the data.

See [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md) for detailed explanation.

### Q: Should I use STARK or native precompiles?

**A:** Depends on your requirements:

| Need | Use |
|------|-----|
| Fast response (3-5 sec) | Native precompile |
| Trustless verification | STARK |
| Cross-chain usage | STARK |
| Low operational cost | Native precompile |
| Creditcoin-only | Native precompile |
| Universal verification | STARK |

See [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) for comprehensive comparison.

### Q: How much does it cost to run?

**A:**

**STARK approach:**
- Proof generation: 15 minutes per proof
- Infrastructure: $10,000+/month (prover servers)
- Per-proof cost: $0.50-1.00
- Gas cost: 2-5M gas

**Native precompile approach:**
- Proof generation: <100ms per proof
- Infrastructure: $100-500/month (simple API server)
- Per-proof cost: <$0.001
- Gas cost: 1.5-2.5M gas

See [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) for detailed breakdown.

### Q: Can users submit their own proofs?

**A:** Yes, but it's poor UX. Users would need to:
- Run proof generation software
- Have access to source chain RPC
- Pay gas costs for submission
- Understand the technical process

For better UX, use a relayer service or prover network.

See [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) for different architecture options.

---

## System Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                  SOURCE CHAIN (ETHEREUM)                 │
│                                                           │
│  Smart Contracts emit events                             │
│  Block data available via RPC                            │
└───────────────────┬─────────────────────────────────────┘
                    │
                    │ RPC Request (REQUIRED)
                    │ eth_getBlockByNumber
                    │
┌───────────────────▼─────────────────────────────────────┐
│              OFF-CHAIN COMPONENT                         │
│              (Relayer / Prover Service)                  │
│                                                           │
│  - Monitors events                                       │
│  - Fetches block data                                    │
│  - Builds merkle trees                                   │
│  - Generates proofs (STARK or native)                    │
│  - Submits to target chain                               │
└───────────────────┬─────────────────────────────────────┘
                    │
                    │ Submit proof + data
                    │
┌───────────────────▼─────────────────────────────────────┐
│                 CREDITCOIN3 CHAIN                        │
│                                                           │
│  ┌────────────────────┐    ┌──────────────────┐         │
│  │ Native Precompile  │    │ STARK Verifier   │         │
│  │ (Fast, <100ms)     │    │ (Trustless, slow)│         │
│  └────────────────────┘    └──────────────────┘         │
│                                                           │
│  Smart contracts receive verified data                   │
└───────────────────────────────────────────────────────────┘
```

**Key Point:** The off-chain component is always required. You cannot eliminate it with either STARK or native precompiles.

---

## Reading Order by Role

### Smart Contract Developer
1. [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) - How to integrate
2. [BLOCK_DATA_FLOW_DIAGRAMS.md](./BLOCK_DATA_FLOW_DIAGRAMS.md) - Visual understanding
3. [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Which to use

### Infrastructure Engineer
1. [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md) - Complete flow
2. [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) - Implementation
3. [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Cost analysis

### Core Protocol Developer
1. [WHAT_IS_BEING_PROVEN.md](./WHAT_IS_BEING_PROVEN.md) - Security properties
2. [QUERY_HASH_SIMPLIFICATION.md](./QUERY_HASH_SIMPLIFICATION.md) - Internal details
3. [WHY_FELTS_NOT_BYTES.md](./WHY_FELTS_NOT_BYTES.md) - Design decisions

### Product Manager / Decision Maker
1. [STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md](./STARK_VS_NATIVE_PRECOMPILE_ANALYSIS.md) - Strategic decision
2. [PROOF_GENERATION_FLOW.md](./PROOF_GENERATION_FLOW.md) - High-level understanding
3. [AUTOMATIC_QUERY_GENERATION_GUIDE.md](./AUTOMATIC_QUERY_GENERATION_GUIDE.md) - Implementation options

---

## Contributing

When adding new architecture documentation:

1. **Place it in this directory**: `docs/architecture/`
2. **Update this README**: Add your document to the appropriate section
3. **Follow the format**: Include "What it covers" and "Read this if" sections
4. **Link to related docs**: Cross-reference other relevant documentation
5. **Add visual diagrams**: When possible, include ASCII or Mermaid diagrams

---

## Additional Resources

- [Creditcoin3 Main README](../../README.md)
- [Prover Service Documentation](../../prover/README.md)
- [Verifier Core Documentation](../../common/verifier-core/README.md)
- [Cairo Programs](../../cairo/)

---

## Glossary

- **Query**: A request to prove specific data from a source chain block
- **Merkle Proof**: Cryptographic proof that a transaction is included in a block
- **Continuity Chain**: Chain of attestations proving block finality
- **Attestation**: Signed statement by validators about a block's state
- **STARK**: Scalable Transparent Argument of Knowledge (cryptographic proof system)
- **Native Precompile**: Built-in blockchain function for efficient computation
- **Relayer**: Off-chain service that fetches and submits data
- **Prover**: Service that generates cryptographic proofs
- **Verifier**: On-chain component that validates proofs

---

Last Updated: January 2025
