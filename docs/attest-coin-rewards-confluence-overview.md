# Attest-coin rewards — overview (Confluence)

**Audience:** product, operations, and partners who need the “what and why” without implementation detail.  
**Companion:** engineers should use the repository technical specification for exact interfaces and behavior.

---

## Summary

**Attest-coin rewards** tie **Creditcoin attestation activity** to a **liquid ERC-20 token on the EVM**. The chain records **reward points per economic actor (stash)**. When that actor is ready to receive tokens, they **claim** through a fixed **EVM entry point (precompile)**. The claim **moves tokens from a treasury balance** held under that entry point—it does **not** mint new token supply as part of the claim.

---

## Why two layers?

| Concept | Plain language |
|--------|----------------|
| **Reward points (on-chain ledger)** | A running balance of “what you’ve earned” from attestation, keyed to your **stash** (the account that holds the bond and economic stake). This is **not** the ERC-20 balance by itself. |
| **The token (EVM)** | The **tradeable** asset lives in a normal **ERC-20 contract**. Users see balances in wallets and DEX tooling like any other token. |
| **The link** | A successful **claim** subtracts from your **reward points** and triggers a **token transfer** from a **treasury** to your **EVM address**. |

This split lets the protocol **account for work** in the attestation layer while still using **standard EVM tokens** for liquidity.

---

## How rewards build up

1. **Attestation success** — When an attestation is committed and your role counts as an **eligible signer**, the network can **credit reward points to your stash** (via the configured per-signer rule). Multiple attestor identities can map to the **same stash**; points accrue to that **one** stash.
2. **Optional operational / test top-ups** — Governance (or automated tests) may also run a **settlement** that **splits a pool** across everyone registered in the attestation ledger. That path exists for **ops and testing**; it is not a substitute for the normal attestation-driven accrual story.

Reward points are **per stash** and are tracked **separately** from ERC-20 balances until someone claims.

---

## How a user gets tokens (claim)

1. **Check points** — The user (or a dApp) reads how many **unclaimed points** the stash has (exposed as an EVM **view** so wallets and apps can show it).
2. **Choose amount** — The user chooses **how many points to convert** in this transaction, up to their remaining balance.
3. **Authorize with a Substrate key** — The **stash account**, or in typical setups its **controller**, signs an off-chain message that binds: **stash**, **claim counter**, **chain domain**, **amount**, and **EVM recipient address**. This uses the same family of keys used elsewhere on Creditcoin (sr25519).
4. **Submit on EVM** — The user sends a transaction **from the EVM address that should receive the tokens**. That address must **match** the recipient named in the signed message—this stops simple front-running of someone else’s payout.
5. **Settlement** — If everything checks out, the system **reduces** the stash’s reward points, **bumps** a per-stash **claim counter** (so old signatures can’t be reused), and the **ERC-20 contract** transfers tokens **from the treasury** to the user’s address.

**Important:** The precompile **does not mint** the ERC-20 on claim. **Treasury must already hold** enough tokens. Funding that treasury is a **governance / launch** responsibility (mint to the treasury address, transfer in, etc.).

---

## Roles in one sentence

| Role | Role |
|------|------|
| **Stash** | The account that earns **reward points** and (with a valid signature) authorizes **claims**. |
| **Controller** | Often the same keys as the stash; if staking uses a separate controller, **either** stash **or** controller may sign, matching common Substrate patterns. |
| **EVM recipient** | The **Ethereum-style address** that actually receives the ERC-20; must be the **sender** of the claim transaction. |
| **Treasury (precompile-held balance)** | The **ERC-20 balance** attributed to the attest-coin **precompile address**—tokens sit there until claims **transfer** them out. |

---

## Governance and operations

- **Which ERC-20 counts** — Network governance (root) points the runtime at a **specific ERC-20 contract** for attest-coin. Until that is set, claims are not meaningful.
- **Treasury funding** — Ensure the precompile’s address on that token holds **enough balance** to cover outstanding reward points you intend to honor (same numeric semantics as points in the current design).
- **Claim ordering** — Each stash has a **monotonic claim counter**. Wallets and scripts must use the **current** counter when building signatures; reusing an old one fails by design.

---

## Security and abuse notes (non-technical)

- **Recipient = sender on EVM** — You cannot claim to someone else’s address in the same transaction you sign for yourself without their cooperation; the design expects the **recipient wallet** to submit the transaction.
- **Replay protection** — The claim counter and signed fields stop **re-submitting** the same approval.
- **Domain separation** — The signed message includes a **chain / domain** field so the same key material is not accidentally reused across unrelated contexts.

---

## Out of scope in this overview

- Exact byte layouts, contract addresses, gas, and RPC methods — see the **technical specification** in the repository.
- Future ideas such as **native-asset** representation on Substrate when depositing to the treasury—possible product direction, not required for the basic claim path described here.

---

## Glossary

| Term | Meaning here |
|------|----------------|
| **Accrued / reward points** | Off-chain–visible **ledger** balance before claim; not the same as ERC-20 wallet balance until claimed. |
| **Claim** | One atomic step: **verify authorization**, **update ledger**, **move ERC-20** to the user. |
| **Treasury** | The **ERC-20 balance** held by the attest-coin precompile address, used to pay claims. |

---

*For implementation details (precompile address, function signatures, signing layout, settlement hooks), see `docs/attest-coin-rewards-precompile-spec.md` in this repository.*
