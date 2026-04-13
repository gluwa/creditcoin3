# Attest-coin rewards: ERC-20 surface + precompile — specification

This document specifies how **attest-coin** integrates with **Creditcoin-native attestation rewards**. The **chosen architecture** is in **§0**; later sections cover **goals**, **fair distribution**, **API**, **linking**, and **open items**.

Runtime details (exact pallets, storage keys, Frontier hooks) must stay aligned with this repository’s Frontier fork and governance process.

---

## 0. Chosen architecture (implementation snapshot)

This section is the **source of truth** for what the runtime + precompile implement today.

### 0.1 Economic layers

| Layer | Role |
|--------|------|
| **ERC-20 (EVM)** | The liquid token. **Supply is not minted by the precompile on claim.** Claims move tokens **out of the precompile’s ERC-20 balance** via standard **`transfer`**, with the subcall’s `msg.sender` equal to the precompile address (treasury **holds** the ERC-20). |
| **Runtime (`pallet-attest-coin-rewards`)** | Tracks **`Accrued[stash]`** (reward **units**, same decimal semantics as the ERC-20, typically 1e18-style), **`ClaimNonce[stash]`**, and the configured **ERC-20 contract address** (`set_attest_coin_token`, root). Epoch settlement **credits** `Accrued` from a configured pool (policy); separate from ERC-20 `totalSupply`. |
| **Deposit → substrate mint (future)** | *Product direction:* at **ERC-20 inception**, nothing is minted on Substrate; when value is **deposited into the precompile** (treasury), a matching **native** representation may be minted (e.g. `pallet-assets`). *Not required for the claim path below.* |

### 0.2 Reward flow (intended)

1. **Per epoch (policy):** participation / vote counters (or other signals) feed **settlement**, which increases **`Accrued[stash]`** and may move value from a protocol treasury into stash-facing balances (exact split: governance; **equal split across ledger stashes** is the current placeholder in code).
2. **Withdraw to EVM:** the stash (or its **controller**, see §0.4) authorizes a **claim** that debits **`Accrued`**, increments **`ClaimNonce`**, and **transfers** ERC-20 from the precompile to the user’s EVM address.

### 0.3 Precompile address

- Fixed precompile **H160** at runtime mapping **`hash(4052)`** → `0x0000000000000000000000000000000000000fd4` (same pattern as other Gluwa precompiles; see `runtime/src/precompiles.rs`).

### 0.4 Claim authorization (EVM)

Mutating entrypoint (Solidity interface in `precompiles/metadata/sol/attest_coin.sol`):

```solidity
function claim(
    bytes32 stash,
    uint256 nonce,
    uint256 chainKey,
    uint256 amount,
    address evmRecipient,
    bytes32 sigHi,
    bytes32 sigLo
) external;
```

- **`evmRecipient` must equal `msg.sender`** (front-running protection).
- **`sigHi` || `sigLo`** is a **64-byte sr25519** signature over the **exact** preimage built by `pallet_attest_coin_rewards::Pallet::claim_signing_message` (see §0.5).
- Verification tries **stash** public key (from `AccountId` **SCALE-encoded** bytes), then **`pallet_staking::Bonded::get(stash)`** (controller), matching typical **stash / controller** usage.

### 0.5 Signing preimage (`claim_signing_message`)

Canonical byte string (must match wallets and the runtime bit-for-bit):

1. UTF-8 prefix: `AttestCoin:claim:v1:`
2. **`AccountId` encoding**: `stash.encode()` (SCALE; 32 bytes for standard `AccountId32`).
3. **`nonce`**: `u64` **little-endian** (8 bytes). Must equal **`ClaimNonce[stash]`** on-chain before the claim succeeds; then incremented.
4. **`chain_key`**: `u64` **little-endian** (8 bytes). Domain separation (e.g. attestation `ChainKey`); `uint256` in calldata must fit in 64 bits.
5. **`amount`**: `u128` **little-endian** (16 bytes). Must not exceed **`Accrued[stash]`** for that claim.
6. **`evm_recipient`**: raw **20 bytes** (no padding inside the preimage).

**Selector** for `claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32)`: **`0x1ffb7a3d`**.

### 0.6 EVM subcall (ERC-20)

- Uses **`transfer(address,uint256)`** (`0xa9059cbb`), **not** `mint`.
- Nested **`CALL`** uses `Context { caller: precompile_address, address: token, apparent_value: 0 }` so **`msg.sender`** on the token is the precompile (treasury must hold balance).

**Frontier note:** `fp-evm` / `evm` 0.42 exposes `PrecompileHandle::call(..., context: &Context)`; the **precompile** sets **`context.caller`** to its **`code_address()`** so the ERC-20 sees the precompile as `msg.sender`. Keep nesting shallow (precompile → token only). Claims must use a normal **`CALL`**, not **`STATICCALL`**, so `transfer` can execute.

### 0.7 Views

- **`accrued(bytes32)`** — `bytes32` is the raw **32-byte** `AccountId` (left-padded as usual in ABI). Returns unclaimed **runtime** accrued units (see **§3** and implementation).

---

## 1. Goals and constraints

| Constraint | Implication |
|------------|-------------|
| Token is a **normal ERC20** deployed off-the-shelf (e.g. OpenZeppelin). | Balances live under the token contract’s H160; **Creditcoin’s native runtime** cannot change them without an **allowed** on-chain entrypoint (`mint`, `transferFrom` treasury, etc.). |
| **No `pallet_assets` required for the claim path** | Canonical reward **accrual** lives in **FRAME storage** (`pallet-attest-coin-rewards`); the ERC-20 is the **liquid** representation after **`transfer`** from treasury. Optional future **native** asset mint on deposit is described in **§0.1**. |
| **Treasury funding** | The precompile **H160** must **hold** ERC-20 balance (minted or transferred to it at inception / by governance). Routine claims **do not mint**; they **`transfer`** out. |
| Attestors use **native Creditcoin account keys** first | Claim flow binds **stash** (and optionally **controller**) via **sr25519** over **§0.5**; **`evmRecipient`** in the preimage must match **`msg.sender`** on the EVM tx. Rewards accrue to the **stash** when one stash backs **multiple** attestor identities (**§2.0.1**). |

---

## 2. Fair distribution when not every attestor appears on-chain each round

### 2.0 Equal stake and unweighted votes

In the current protocol, **votes are not weighted**: every active attestor is treated as having the **same stake**, and there is **no** per-attestor multiplier based on bonded amount for a given vote.

**Implications for rewards**

- **Base unit of fairness** is “**one attestor, one unit of eligible work**” for a given duty (e.g. one eligible attestation opportunity), not “proportional to stake.”
- **Sampling fairness** (§2.1–2.3) is the main distortion to fix: two honest attestors with the same role should have the **same expected** payout over time, even if inclusion is random.
- **Accounting is simpler** than in stake-weighted systems: no need for reward curves indexed by stake class; focus on **who was eligible**, **what was observed on-chain**, and **epoch/era budgets** (§2.5).

### 2.0.1 Stash as controlling account (accrual key)

In Creditcoin’s attestation model, each registered attestor carries a **`stash: AccountId`** (bonded funds and economic stake live against that account). It is **possible for one stash to control multiple attestor identities** (e.g. distinct operator keys or chain roles under the same economic actor).

**Recommendation for attest-coin accrual**

- **Accrue rewards to the stash**, not to each attestor row separately, when the same stash backs more than one active attestor identity. Operationally, **work performed by any identity keyed to `stash` S** increases **`Accrued[S]`** (subject to the fairness rules in §2.1–2.5).
- **Why**: the stash is the **controlling account** for bond, slashing, and operational funding; paying the “operator” per identity can **split** liquidity awkwardly across keys that share one economic owner. A single **`Accrued`** balance per stash matches **claim once** to an EVM beneficiary per **§0**.
- **Alternative (per-attestor accrual)**: still valid if product wants **per-key** payouts; then fairness and sampling corrections apply **per attestor id**, and the same stash may hold **several** independent reward buckets. This is **more** state and more claims.

**Implementation note**

- Attribution still uses **observable attestor activity** (which identity signed which checkpoint); settlement **aggregates** into `Accrued[stash]` by mapping each included vote to its attestor’s `stash` before summing.

---

### 2.1 Problem

Operationally, **every** attestor may **cast** a vote (off-chain or network gossip), but the **subset** that ends up **included in the block / observed by the runtime** can be **partial** or **sampled** (randomness, bandwidth, producer choice, etc.). If rewards are naïvely tied only to “included” votes, participants with equal honest work can receive **unequal** payouts.

### 2.2 Principles

1. **Define the reward unit**  
   Clarify what is rewarded: e.g. “eligible vote cast,” “signature verified,” “weight in committee,” etc. The spec should match **cryptographic or protocol** definitions, not only “included in block.”

2. **Separate observability from inclusion**  
   If the runtime only sees a sample, either:
   - **Bring more evidence on-chain** (aggregates, commitments, Merkle batches) when cost allows, or  
   - **Use statistical fairness** over time (below).

3. **Avoid raw inclusion as the only score**  
   Tie accrual to **long-horizon** metrics so short-run sampling noise averages out.

### 2.3 Mechanisms (combinable)

| Mechanism | Idea |
|-----------|------|
| **Windowed accrual** | Over an **epoch** of `N` blocks/slots, track **attempted** vs **included** participation (per attestor identity, then **aggregate to stash** if using §2.0.1) using whatever the runtime actually observes; pay **expected** reward rate × **observed inclusion probability** estimated from the window. |
| **Inverse probability weighting** | If selection probability `p` is known (VRF, documented policy), accrue **reward / p** per eligible action so **expected** payout matches full participation. |
| **Committee quotas** | Cap per-epoch variance: each **stash** (or each attestor identity) has a **target** number of on-chain appearances; excess rewards redistributed or banked. |
| **Slashing / quality** | Penalize missed slots only when provably attributable; don’t conflate network randomness with malice. |
| **Transparency** | Publish the **sampling rule** and, where possible, on-chain **randomness** (VRF output) so “fairness” is verifiable. |

### 2.4 Product decision

The exact formula is a **governance + cryptoeconomics** choice. The implementation should store reward liability **in Creditcoin** (native runtime storage) using a rule agreed from §2.3. Prefer **`Accrued[stash]`** (see §2.0.1) unless product explicitly requires **per-attestor-id** buckets. Realize value **on claim** per **§0** (debit `Accrued`, ERC-20 **`transfer`** from treasury).

---

### 2.5 How rewards are defined and when balance accrues (proposals)

**Terminology (align with implementation)**

- **Babe epoch** — VRF randomness and BABE slot cadence (see attestation tests around `EpochDuration` / `epoch_index`). Often used for **policy ticks** (elections, parameter updates).
- **Attestation / protocol epoch** — High-level period used in attestation events (e.g. `epoch: u64` in pallet events). May be aligned with Babe or a multiple of it.
- **Staking era** — If the chain uses `pallet_staking`, `CurrentEra` and **era** boundaries may be a natural **settlement** cadence for rewards (attestation ledger already uses `EraIndex` for **unlock** timing).

Rewards need: (1) a **budget** (where tokens come from—inflation, treasury, fixed cap per period), (2) a **splitting rule** among **stashes** or among attestor identities (equal per unit work under §2.0), (3) a moment when **`Accrued[stash]`** (recommended; §2.0.1) or **`Accrued[attestor_id]`** is updated in storage.

Below are **mutually comparable proposals**; pick one primary model and optionally combine (e.g. epoch budget + per-observation micro-accrual capped by epoch budget).

#### Proposal A — Budget per period, settle at period end (recommended for clarity)

- **Definition**: Governance sets **`R_period`**: total attest-coin (in smallest units) to allocate per **period** `P` (e.g. one **Babe epoch**, one **attestation epoch**, or one **staking era**).
- **Split**: Among **stashes** (or attestor ids, if not aggregating) **eligible** in `P`, each receives a **share** of `R_period`. With **equal stake** (§2.0), the natural split is **equal per qualifying work unit**:
  - either **equal split** of `R_period` among all **stashes** that met a **minimum on-chain liveness** threshold in `P` (with work summed across identities sharing a stash per §2.0.1), or
  - **weighted by corrected work units** using §2.3 (e.g. inverse inclusion probability) so **expected** pay is equal for equal honesty.
- **When accrual is written**: At the **last block of** `P` (or first block of `P+1`), a hook (**`on_finalize`**, **`on_idle`**, or a **permissionless `close_period` extrinsic`) **commits** `Accrued += share` for each **stash** (or each attestor id, per policy). Until then, **`Accrued` does not increase** for that period (only **pending** internal counters might).
- **Pros**: Simple monetary policy (“`X` tokens per era”); easy audits. **Cons**: Attestors wait until period end to see on-chain accrual (still **claim** later via **§0** / **§3**).

#### Proposal B — Continuous accrual on each on-chain observation

- **Definition**: A constant **`r`** (smallest units) credited **every time** the runtime **accepts** an attestor’s contribution for a checkpoint (or other observable event).
- **When accrual is written**: **Immediately** in the dispatch path that records the vote/checkpoint (same block as inclusion).
- **Budget**: Set **`r`** from an inflation curve, or enforce a **per-period cap**: sum of accruals in `P` cannot exceed `R_period` (remainder rolls to treasury or next period).
- **Pros**: Smooth, visible progress block-by-block. **Cons**: Needs careful **caps** so total issuance matches policy; sampling bias still requires §2.3 if not everyone is observed every time.

#### Proposal C — Staking-era payout mirror (if tied to existing staking economics)

- **Definition**: Reuse **`EraPayout`-style** logic: a **curve** maps total issuance per **staking era** to attestor rewards (similar to validator rewards), but **split among attestors** using **equal per work unit** (§2.0) rather than stake weight.
- **When accrual is written**: Typically at **era end** when staking payouts are computed, or in the **same block** as `pallet_staking` reward distribution (ordering must be deterministic).
- **Pros**: One **era** for stakers and attestors. **Cons**: Couples attestor rewards to staking schedule even if attestation cadence differs.

#### Proposal D — Deferred / lazy accrual at claim time (not ideal as sole model)

- **Definition**: Store only **immutable** on-chain events or commitments; **`Accrued`** is computed when the user **claims** from historical data.
- **Pros**: Less state if events are few. **Cons**: Heavy client/runtime replay; harder light-client proofs; usually **avoid** unless combined with Merkle checkpoints.

#### Summary: when does “balance” accrue?

| Model | When `Accrued[stash]` (or `Accrued[attestor]`) increases | ERC-20 movement |
|-------|-----------------------------------|----------------------|
| A — Period settlement | End of period (or start of next) | On **claim** (**`transfer`** from treasury per **§0**) |
| B — Per observation | Each included/accepted attestation | On **claim** |
| C — Era-linked | Era boundary (with staking) | On **claim** |
| D — Lazy | At claim (computed) | On **claim** |

**Recommendation**: Start with reward **Proposal A** or **Proposal B** (in the table above) with an explicit **period cap** so issuance is predictable; use §2.3 adjustments so **sampling** does not break **equal expected pay** under §2.0.

---

## 3. Precompile API (recommended surface)

**Implemented surface** is fixed in **§0.4–§0.7**. The list below is the **normative** ABI for integrators.

### 3.1 Views (implemented)

- **`accrued(bytes32 stash)`**  
  Returns **`Accrued[stash]`** from **`pallet-attest-coin-rewards`** (unclaimed runtime units).  
  Selector: **`0xf92f23a7`** (see `precompiles/attest-coin`).

### 3.2 Mutations (implemented)

- **`claim(bytes32 stash, uint256 nonce, uint256 chainKey, uint256 amount, address evmRecipient, bytes32 sigHi, bytes32 sigLo)`**  
  - **`evmRecipient == msg.sender`** (required).  
  - **sr25519** signature over **`claim_signing_message`** (§0.5).  
  - On success: **`commit_claim`** (debit `Accrued`, bump nonce), then ERC-20 **`transfer(evmRecipient, amount)`** from treasury.  
  - Selector: **`0x1ffb7a3d`**.

### 3.3 Not implemented in precompile (optional / future)

- **`claimableRewards`**, **`vestingInfo`**, **`claimTo`**, **vesting** — not part of the current precompile; can be added later without changing §0.5 preimage if versioned with a new prefix.

### 3.4 Historical note (mint-on-claim)

Earlier drafts described **`claim(uint256)`** + ERC-20 **`mint`**. That has been **replaced** by **§0** (**treasury + `transfer` + signed claim**).

---

## 4. Attestors without EVM keys: linking and claims

### 4.1 Facts

- Attestors authenticate to Creditcoin with **native accounts** (`AccountId`, sr25519/ed25519 as configured).
- Frontier maps **H160 → AccountId** via **`AddressMapping`** (see `pallet_evm` config). There is **no** automatic equality between an attestor’s `AccountId` and “their” H160 unless **explicitly linked**.

### 4.2 Recommended model

1. **Registration extrinsic (Creditcoin native)** (optional product feature)  
   `link_evm_beneficiary(evm_address)` signed by the **account that controls rewards**—typically the **stash** when **`Accrued[stash]`** is used (§2.0.1), not each separate attestor identity. Stored in pallet storage if implemented.

2. **Claim path A — Creditcoin-native first**  
   Extrinsic **`claim_rewards_attest_coin()`** (if added): signed by the **`AccountId`** that owns **`Accrued`**, then runtime performs the EVM leg. Not required if **§0** EVM path is sufficient.

3. **Claim path B — EVM transaction after link**  
   User sends tx from **linked H160**; precompile checks `AddressMapping` / link table. **§0** uses **signed preimage** instead of link-table checks.

4. **Claim path C — signature inside precompile (implemented)**  
   **EVM transaction** to the precompile with **`claim(...)`** and **sr25519** over **§0.5**. **Stash** (or **controller**) signs; **`evmRecipient == msg.sender`**.

### 4.3 Security notes

- **Linking** (if used) must be **replay-protected** (nonce, chain id, pallet nonce).
- **Changing beneficiary** should be **stash-** (or reward-owner-) signed or time-locked per policy.
- **Precompile `view` methods** can expose accrual **by `AccountId` hash** (stash or attestor id, per §2.0.1); **mutations** enforce **§0**.

---

## 5. Summary table

| Topic | Recommendation / **chosen (§0)** |
|-------|----------------|
| Architecture | ERC-20 **treasury at precompile**; claim uses **`transfer`**, not mint. Optional future: **deposit → precompile** ↔ **native asset** mint (see §0.1). |
| Vote weight | **Unweighted**; **equal stake** per attestor → equal base credit per eligible work unit; combine with §2.3 for sampling fairness. **Placeholder settlement:** equal split of epoch pool across `Ledger` keys. |
| Token | Plain ERC-20; treasury must **hold** balance; **mint** to precompile only for funding (governance / inception), not per-claim inflation. |
| Accrual | Runtime **`Accrued[stash]`**; **claim nonce** per stash; **cadence** aligned with Babe epoch length for settlement hook (configurable constant). |
| Fairness | §2.0–2.3 + budget model in §2.5 (still governance). |
| Precompile | **View**: `accrued(bytes32)`; **Mutate**: **`claim(...)`** + **sr25519** + **`transfer`**. |
| Inner ERC-20 call | Precompile **`CALL`** with **`caller = precompile`**, **`transfer`** (see §0.6). |
| EVM claim | **Signed claim** (**§0.4–0.5**); **stash or controller** sr25519; **`evmRecipient == msg.sender`**. |

---

## 6. Open items

### 6.1 Closed or partially closed

- [x] **Architecture direction (this branch):** treasury-held ERC-20 + runtime **`Accrued`** + **`transfer`** on claim + **sr25519** signed preimage (**§0**).
- [x] **`Accrued[stash]`** as accrual key (aligned with §2.0.1).
- [x] **Frontier:** precompile → ERC-20 nested **`CALL`** with controlled **`Context.caller`** (**§0.6**); **transfer** instead of mint for claims.
- [x] **Decimals / units:** reward **`RewardPoints`** and ERC-20 use the same **1e18-style** numeric range in implementation (fits `u128`); enforce in policy.
- [x] **EVM claim binding:** **`evmRecipient == msg.sender`** + preimage includes **stash / nonce / chain_key / amount / recipient bytes**.

### 6.2 Still open (governance / product)

- [ ] **Equal-stake** vs future **weighted** votes (§2.0).
- [ ] **Reward cadence** (Babe epoch vs attestation epoch vs staking **era**) vs current **`EpochDuration`** hook only.
- [ ] **Proposal A/B/C** (§2.5): exact **`R_period` / `r` / caps**; replace **equal split** placeholder with **vote counters → settlement → `Accrued`** as in product discussions.
- [ ] **Exact fairness** formula vs attestation sampling (§2.3–2.5).
- [ ] **Deposit → `pallet-assets` mint** when ERC-20 is deposited to precompile (§0.1) — not yet wired in code.
- [ ] **Governance:** `set_attest_coin_token` (root today); emergency pause; treasury top-up policy.
- [ ] **Gas / weight** limits for batched claims and heavy settlement paths.

### 6.3 Tooling

- [ ] Regenerate **Polkadot.js** types / metadata after runtime upgrades so `attestCoinRewards.*` queries match production nodes.
- [ ] Wallet **signing** helpers for **`claim_signing_message`** (exact bytes in §0.5).
