# Attest-coin rewards: ERC-20 surface + precompile — specification (Options A & B)

This document specifies how **attest-coin** can integrate with **Creditcoin-native attestation rewards** in two complementary shapes: **Option A** (plain Solidity ERC-20 + precompile as minter, no native fungible pallet) and **Option B** (canonical native asset + **wrapped** attest-coin on EVM with a lock/mint invariant). It also covers **fair distribution**, **API shape**, and **attestor identity (Creditcoin native account vs EVM)**.

It is a design spec: runtime details (exact pallets, storage keys, Frontier hooks) must be validated against this repository’s Frontier fork and governance process.

---

## 1. Goals and constraints

| Constraint | Implication |
|------------|-------------|
| Token is a **normal ERC20** deployed off-the-shelf (e.g. OpenZeppelin). | Balances live under the token contract’s H160; **Creditcoin’s native runtime** cannot change them without an **allowed** on-chain entrypoint (`mint`, `transferFrom` treasury, etc.). |
| **No `pallet_assets`** (Option A) | Canonical reward **accrual** should live in **FRAME storage** (attestation pallet or a dedicated rewards pallet); the ERC-20 is the **liquid** representation after mint. |
| **Option A** (§2) | A **fixed precompile address** is the **trusted minter** (or one of them) for a **deployed** ERC-20 contract. |
| **Option B** (§3) | Canonical **supply** lives on the **native** side (balances or a future native fungible asset); EVM holds **only wrapped** attest-coin, **1:1** backed by **locked** native balance in protocol escrow. |
| Attestors use **native Creditcoin account keys** first | Claim/beneficiary flow must define **binding** between `AccountId` and optional **H160** (wallet). Rewards may accrue to the **stash** (controlling account) when one stash backs **multiple** attestor identities (§4.0.1). Option B adds explicit **wrap** authorization to an EVM address. |

---

## 2. Option A — precompile as authorized minter (in depth)

### 2.1 Roles on the ERC20

- **Owner / admin** (e.g. multisig or “boss”): deploys the contract, configures pausing, upgrades if upgradeable, and **grants roles**.
- **Minter** (protocol): the only address that routine reward minting trusts. In Option A, **`MINTER_ROLE` (or equivalent) is granted to the precompile’s H160**, published in the chain spec **before** mainnet deploy so the token contract can be deployed with that address in mind.

The precompile address is **not** a private key; it is a **well-known execution entry** at `0x000…0XXX` (same pattern as existing precompiles in this repo).

### 2.2 End-to-end flow

1. **Accrual (on Creditcoin, native runtime)**  
   As attestors perform work, the runtime updates **accrued** balances in pallet storage. The **accrual key** should usually be the **stash** `AccountId` (controlling account) when one stash funds multiple attestor identities—see **§4.0.1**. Claims and EVM linking are then authorized from that account (see §6). No ERC20 supply change yet (or supply changes only on claim—product choice).

2. **Claim initiation**  
   A user (or automation) triggers **mint** of the ERC20 **only** through paths that the runtime allows:
   - **Primary**: call the **precompile** (e.g. `claim` / `claimTo`), which:
     - Verifies entitlement from **Creditcoin** runtime storage (and optional vesting rules).
     - Updates accrual/claimed state **on Creditcoin** (native runtime).
     - Performs an **EVM call** into the ERC20 `mint(beneficiary, amount)` (or `mint` + events), with **`msg.sender` equal to the precompile address** so OpenZeppelin-style `onlyRole(MINTER)` passes.

3. **Inner EVM call from precompile**  
   In **this** repo’s dependency stack, nested calls **are** supported; see **§2.2.1**. You may still prefer §2.4 for policy reasons (simpler auditing, avoiding deep call stacks).

### 2.2.1 Frontier / `evm` stack (verified for this repository)

**Sources**

- `fp-evm` comes from [`gluwa/frontier_2`](https://github.com/gluwa/frontier_2), branch `stable2409_patch`, commit `49bcd9779d851b0c6dab5afef59c48d5dfaf8e68` (see root `Cargo.lock`).
- `fp-evm` re-exports `PrecompileHandle` from the Rust **`evm`** crate; the resolved version in this workspace is **`evm` 0.42.0** (crates.io).

**What `PrecompileHandle` provides**

In `evm` 0.42, `PrecompileHandle` includes:

```text
fn call(
    &mut self,
    to: H160,
    transfer: Option<Transfer>,
    input: Vec<u8>,
    gas_limit: Option<u64>,
    is_static: bool,
    context: &Context,
) -> (ExitReason, Vec<u8>);
```

with `Context { caller, address, apparent_value }` (`evm-runtime`). The doc comment on the trait states that the **precompile chooses the context** for the subcall.

**Making `msg.sender == precompile` on the ERC20**

For a `CALL` into the token contract, the executing account’s `msg.sender` is **`context.caller`**. So the attest-coin precompile should **not** forward the user’s `handle.context().caller` if the token’s `mint` is **`onlyRole(MINTER)`** for the precompile address. Instead, build a subcall context such as:

- `caller` = `handle.code_address()` (the precompile’s own address), and  
- `address` = ERC20 contract address,  
- `apparent_value` = `0` (unless you also send value).

That matches OpenZeppelin-style **`MINTER_ROLE` granted to the precompile H160**.

**Caveats (still read these before shipping)**

- **Call depth / traps**: `evm`’s `StackExecutorHandle` implementation notes that resolving nested **traps** via recursive execution can risk **stack overflow** if precompiles chain subcalls carelessly—keep nesting shallow (precompile → ERC20 only).
- **Gas**: Subcalls record opcode-level gas; precompile code must pass a sensible `gas_limit` and handle `ExitReason` failures.
- **`STATICCALL`**: If the **outer** EVM path into the precompile is static, state-changing `mint` in the callee may be rejected—ensure claims run in a normal `CALL`, not `STATICCALL`.
- **`precompile_utils` / macros**: Your project uses helpers that may wrap `handle`; ensure generated code exposes or forwards **`call`** with the **custom** `Context` above (not an accidental default).

**Conclusion**: Nested precompile → ERC20 **`CALL`s are supported** by the **`evm`** engine version pinned here; **`msg.sender == precompile`** is achievable by **setting `Context.caller` to `code_address`**. Use §2.4 if you want minting via **`Runner::call`** from a pallet instead.

### 2.3 Why this pattern

- **Single economic truth for “who can inflate supply”**: the ERC20 explicitly lists the precompile as minter; no opaque root key minting from random EOAs.
- **Auditable surface**: reward rules live in Rust + storage; the token contract stays a standard ERC20.

### 2.4 Fallback: pallet + `Runner::call` (same security story)

If you **choose not** to use nested precompile `CALL`s (audit preference, tooling limits, or static-call pitfalls), or you mint from a **Creditcoin-only** extrinsic (native runtime, not EVM):

- Grant **`MINTER_ROLE` to a protocol H160** that exists only as a **`from` address** in privileged `Runner::call` invocations (no hot wallet), **or** keep minter on precompile and use a **small shim contract** at that H160 (advanced).

This repository’s runtime already exposes patterns using `Runner::call(from, to, data, …)` (see `runtime/src/lib.rs`). A **rewards settlement extrinsic** (root/council/permissioned) or an **on_finalize hook** could call `mint` with `from = PROTOCOL_H160`. The spec still recommends **one** canonical minter address published in genesis/spec.

---

## 3. Option B — native asset + wrapped attest-coin on EVM (lock / mint)

This is the design discussed as “substrate asset + wrapped on EVM”: **canonical balances and inflation/rewards** live in the **Creditcoin native runtime**; the **EVM** only ever holds a **wrapped** token whose supply is **fully backed** by native funds locked in a **protocol-controlled escrow** (often described as **keyless**: no human-held private key, only runtime rules).

It does **not** require the wrapped token to be a hand-written Solidity **contract**; the **ERC-20–compatible** interface (balances, `Transfer`, `Mint`/`Burn` semantics) can be implemented by a **precompile** so wallets and indexers can track it like any ERC-20.

### 3.1 Core invariant (auditable)

At all times:

**`wrapped_total_supply_on_EVM == native_locked_in_escrow`**

- **Rewards / era points** accrue to **native** accounts (same fairness goals as §4).
- **Wrapping** moves value from a user’s **spendable native** balance into **escrow** and **mints** wrapped tokens to a specified **H160**.
- **Unwrapping** **burns** wrapped tokens and **releases** native balance from escrow to a chosen **native** `AccountId` (exact rules are a product choice).

Anyone auditing the chain can check that **EVM circulating wrapped supply** never exceeds **escrow**, so **supply is explainable** without trusting an opaque multisig.

### 3.2 Where rewards are minted

- **Minting attest-coin as rewards** happens on the **native** side (extrinsic / hook / pallet), consistent with “rewards are attestation-based” and optional **era-style** accounting (§4.5).
- The **EVM** does not mint the canonical asset; it only receives **wrapped** representation when users **wrap**.

### 3.3 Wrap — native → EVM

1. User holds **native** attest-coin (e.g. credited after attestation / era settlement to **`Accrued[stash]`** per §4.0.1).
2. User authorizes **locking** `amount` **and** crediting wrapped tokens to **`evm_beneficiary`**, with replay protection.
3. **Suggested authorization** (CTO sketch): a **native (sr25519) signature** from the **same `AccountId` that holds the balance**—typically the **stash** when rewards accrue to stash—over a structured payload including at least:
   - **`evm_beneficiary`** (`H160`),
   - **`amount`** (wrap limit),
   - **`nonce`** (per-account or global replay guard),
   - **chain / pallet domain** separation so signatures are not reusable elsewhere.
4. Runtime verifies the signature, **transfers** `amount` from the signer’s native balance **into escrow**, and **mints** wrapped attest-coin to `evm_beneficiary` (precompile-backed ERC-20 or contract). A **`Mint`** event (or equivalent) is emitted on the EVM side for trackers.

Implementation can be a **native extrinsic** that performs both legs, or a **precompile** entry that dispatches native logic—either way the **atomic** property is: **lock + mint** in one transaction.

### 3.4 Unwrap — EVM → native

1. User **burns** wrapped attest-coin on EVM (or calls an unwrap precompile that burns and dispatches native release).
2. Runtime **unlocks** the corresponding amount from **escrow** to the user’s **native** `AccountId` (or a linked account). A **`Burn`** event (or equivalent) is emitted on the EVM side.

**Who “unwraps”?** Whoever controls the **wrapped** balance on EVM (same as any ERC-20 transfer): typically the **H160** that received wrapped funds. That address may be **linked** to a native identity (§6) so unlock goes to the correct `AccountId`.

### 3.5 Who wraps? (Clarification)

**Wrapping is not an anonymous EVM-only action.** The party that **locks native** funds must prove control of the **native** account whose balance moves into escrow—hence the **native signature** (or equivalent extrinsic signed by that account). So:

- **Who wraps?** Holders of **native** attest-coin who want **EVM-usable** tokens (e.g. to **pay for proving** in Solidity-accessible flows)—in practice often the **stash** key when **`Accrued[stash]`** is used (§4.0.1). They specify **which EVM address** receives the minted wrapped balance in the signed payload (or via a prior **link** extrinsic + nonce).

### 3.6 Precompile role in Option B

- **Wrapped ERC-20 surface**: implement **`balanceOf` / `transfer` / `approve`** (and **mint/burn** only callable from runtime hooks), or expose **view** methods for **claimable native points** before wrap—same “fetch claimable attestation points” idea as Option A, but **minted** supply on EVM is **only** wrapped stock, not the canonical native float.
- **Views** can still expose **accrued / claimable** native rewards **before** wrap; **wrap** is the step that materializes EVM-side liquidity.

### 3.7 Option A vs Option B — when to pick

| | **Option A** (§2) | **Option B** (this section) |
|---|-------------------|-----------------------------|
| **Canonical supply** | ERC-20 contract + minter role | **Native** balance + escrow |
| **Rewards** | Mint ERC-20 (via precompile / runner) | Mint **native**, then user **wraps** |
| **Audit story** | Minter is precompile / protocol H160 | **`wrapped_supply == escrow`** always |
| **`pallet_assets` / native currency** | Not required | **Fits** a native fungible asset model |

**Recommendation**: If product requires attest-coin to be **first-class on native** (staking, fees, treasury) **and** a **clean EVM** story, Option B is strong. If you want **minimal** native surface and a **single** ERC-20 as source of truth, Option A is simpler.

---

## 4. Fair distribution when not every attestor appears on-chain each round

### 4.0 Equal stake and unweighted votes

In the current protocol, **votes are not weighted**: every active attestor is treated as having the **same stake**, and there is **no** per-attestor multiplier based on bonded amount for a given vote.

**Implications for rewards**

- **Base unit of fairness** is “**one attestor, one unit of eligible work**” for a given duty (e.g. one eligible attestation opportunity), not “proportional to stake.”
- **Sampling fairness** (§4.1–4.3) is the main distortion to fix: two honest attestors with the same role should have the **same expected** payout over time, even if inclusion is random.
- **Accounting is simpler** than in stake-weighted systems: no need for reward curves indexed by stake class; focus on **who was eligible**, **what was observed on-chain**, and **epoch/era budgets** (§4.5).

### 4.0.1 Stash as controlling account (accrual key)

In Creditcoin’s attestation model, each registered attestor carries a **`stash: AccountId`** (bonded funds and economic stake live against that account). It is **possible for one stash to control multiple attestor identities** (e.g. distinct operator keys or chain roles under the same economic actor).

**Recommendation for attest-coin accrual**

- **Accrue rewards to the stash**, not to each attestor row separately, when the same stash backs more than one active attestor identity. Operationally, **work performed by any identity keyed to `stash` S** increases **`Accrued[S]`** (subject to the fairness rules in §4.1–4.5).
- **Why**: the stash is the **controlling account** for bond, slashing, and operational funding; paying the “operator” per identity can **split** liquidity awkwardly across keys that share one economic owner. A single **`Accrued`** balance per stash matches **claim once, link once** to an EVM beneficiary.
- **Alternative (per-attestor accrual)**: still valid if product wants **per-key** payouts; then fairness and sampling corrections apply **per attestor id**, and the same stash may hold **several** independent reward buckets. This is **more** state and more claims.

**Implementation note**

- Attribution still uses **observable attestor activity** (which identity signed which checkpoint); settlement **aggregates** into `Accrued[stash]` by mapping each included vote to its attestor’s `stash` before summing.

---

### 4.1 Problem

Operationally, **every** attestor may **cast** a vote (off-chain or network gossip), but the **subset** that ends up **included in the block / observed by the runtime** can be **partial** or **sampled** (randomness, bandwidth, producer choice, etc.). If rewards are naïvely tied only to “included” votes, participants with equal honest work can receive **unequal** payouts.

### 4.2 Principles

1. **Define the reward unit**  
   Clarify what is rewarded: e.g. “eligible vote cast,” “signature verified,” “weight in committee,” etc. The spec should match **cryptographic or protocol** definitions, not only “included in block.”

2. **Separate observability from inclusion**  
   If the runtime only sees a sample, either:
   - **Bring more evidence on-chain** (aggregates, commitments, Merkle batches) when cost allows, or  
   - **Use statistical fairness** over time (below).

3. **Avoid raw inclusion as the only score**  
   Tie accrual to **long-horizon** metrics so short-run sampling noise averages out.

### 4.3 Mechanisms (combinable)

| Mechanism | Idea |
|-----------|------|
| **Windowed accrual** | Over an **epoch** of `N` blocks/slots, track **attempted** vs **included** participation (per attestor identity, then **aggregate to stash** if using §4.0.1) using whatever the runtime actually observes; pay **expected** reward rate × **observed inclusion probability** estimated from the window. |
| **Inverse probability weighting** | If selection probability `p` is known (VRF, documented policy), accrue **reward / p** per eligible action so **expected** payout matches full participation. |
| **Committee quotas** | Cap per-epoch variance: each **stash** (or each attestor identity) has a **target** number of on-chain appearances; excess rewards redistributed or banked. |
| **Slashing / quality** | Penalize missed slots only when provably attributable; don’t conflate network randomness with malice. |
| **Transparency** | Publish the **sampling rule** and, where possible, on-chain **randomness** (VRF output) so “fairness” is verifiable. |

### 4.4 Product decision

The exact formula is a **governance + cryptoeconomics** choice. The implementation should store reward liability **in Creditcoin** (native runtime storage) using a rule agreed from §4.3. Prefer **`Accrued[stash]`** (see §4.0.1) unless product explicitly requires **per-attestor-id** buckets. Realize value **on claim** in line with the chosen architecture: **Option A** (§2) mints the **ERC-20** after updating accrual; **Option B** (§3) mints **native** rewards first, then users **wrap** to EVM (§3.3) when they want ERC-20–trackable liquidity.

---

### 4.5 How rewards are defined and when balance accrues (proposals)

**Terminology (align with implementation)**

- **Babe epoch** — VRF randomness and BABE slot cadence (see attestation tests around `EpochDuration` / `epoch_index`). Often used for **policy ticks** (elections, parameter updates).
- **Attestation / protocol epoch** — High-level period used in attestation events (e.g. `epoch: u64` in pallet events). May be aligned with Babe or a multiple of it.
- **Staking era** — If the chain uses `pallet_staking`, `CurrentEra` and **era** boundaries may be a natural **settlement** cadence for rewards (attestation ledger already uses `EraIndex` for **unlock** timing).

Rewards need: (1) a **budget** (where tokens come from—inflation, treasury, fixed cap per period), (2) a **splitting rule** among **stashes** or among attestor identities (equal per unit work under §4.0), (3) a moment when **`Accrued[stash]`** (recommended; §4.0.1) or **`Accrued[attestor_id]`** is updated in storage.

Below are **mutually comparable proposals**; pick one primary model and optionally combine (e.g. epoch budget + per-observation micro-accrual capped by epoch budget).

#### Proposal A — Budget per period, settle at period end (recommended for clarity)

- **Definition**: Governance sets **`R_period`**: total attest-coin (in smallest units) to allocate per **period** `P` (e.g. one **Babe epoch**, one **attestation epoch**, or one **staking era**).
- **Split**: Among **stashes** (or attestor ids, if not aggregating) **eligible** in `P`, each receives a **share** of `R_period`. With **equal stake** (§4.0), the natural split is **equal per qualifying work unit**:
  - either **equal split** of `R_period` among all **stashes** that met a **minimum on-chain liveness** threshold in `P` (with work summed across identities sharing a stash per §4.0.1), or
  - **weighted by corrected work units** using §4.3 (e.g. inverse inclusion probability) so **expected** pay is equal for equal honesty.
- **When accrual is written**: At the **last block of** `P` (or first block of `P+1`), a hook (**`on_finalize`**, **`on_idle`**, or a **permissionless `close_period` extrinsic**) **commits** `Accrued += share` for each **stash** (or each attestor id, per policy). Until then, **`Accrued` does not increase** for that period (only **pending** internal counters might).
- **Pros**: Simple monetary policy (“`X` tokens per era”); easy audits. **Cons**: Attestors wait until period end to see on-chain accrual (still **claim** later via §5).

#### Proposal B — Continuous accrual on each on-chain observation

- **Definition**: A constant **`r`** (smallest units) credited **every time** the runtime **accepts** an attestor’s contribution for a checkpoint (or other observable event).
- **When accrual is written**: **Immediately** in the dispatch path that records the vote/checkpoint (same block as inclusion).
- **Budget**: Set **`r`** from an inflation curve, or enforce a **per-period cap**: sum of accruals in `P` cannot exceed `R_period` (remainder rolls to treasury or next period).
- **Pros**: Smooth, visible progress block-by-block. **Cons**: Needs careful **caps** so total issuance matches policy; sampling bias still requires §4.3 if not everyone is observed every time.

#### Proposal C — Staking-era payout mirror (if tied to existing staking economics)

- **Definition**: Reuse **`EraPayout`-style** logic: a **curve** maps total issuance per **staking era** to attestor rewards (similar to validator rewards), but **split among attestors** using **equal per work unit** (§4.0) rather than stake weight.
- **When accrual is written**: Typically at **era end** when staking payouts are computed, or in the **same block** as `pallet_staking` reward distribution (ordering must be deterministic).
- **Pros**: One **era** for stakers and attestors. **Cons**: Couples attestor rewards to staking schedule even if attestation cadence differs.

#### Proposal D — Deferred / lazy accrual at claim time (not ideal as sole model)

- **Definition**: Store only **immutable** on-chain events or commitments; **`Accrued`** is computed when the user **claims** from historical data.
- **Pros**: Less state if events are few. **Cons**: Heavy client/runtime replay; harder light-client proofs; usually **avoid** unless combined with Merkle checkpoints.

#### Summary: when does “balance” accrue?

| Model | When `Accrued[stash]` (or `Accrued[attestor]`) increases | ERC20 `totalSupply` |
|-------|-----------------------------------|----------------------|
| A — Period settlement | End of period (or start of next) | On **claim** (mint), unless you mint to a vault each period |
| B — Per observation | Each included/accepted attestation | On **claim** |
| C — Era-linked | Era boundary (with staking) | On **claim** |
| D — Lazy | At claim (computed) | On **claim** |

**Recommendation**: Start with reward **Proposal A** or **Proposal B** (in the table above) with an explicit **period cap** so issuance is predictable; use §4.3 adjustments so **sampling** does not break **equal expected pay** under §4.0. (Do not confuse with architectural **Option A / Option B** in §2–§3.)

---

## 5. Precompile API (recommended surface)

The following skews toward **Option A** (mint-on-claim to ERC-20). **Option B** can expose the same **views** for **native accrued / claimable** points; **wrap** / **unwrap** become the mutating paths instead of direct mint-to-beneficiary (see §3).

Solidity-style interface (names illustrative; encoding follows project conventions):

### 5.1 Views

- **`accruedRewards(bytes32 creditcoinAccountId)`** (native `AccountId` encoding—typically the **stash** if using §4.0.1; or a compact key type)  
  Returns **unclaimed** accrued amount in reward **units** (matching ERC20 decimals after conversion).

- **`claimableRewards(...)`**  
  Returns **accrued − locked in vesting** (if vesting enabled).

- **`vestingInfo(bytes32 creditcoinAccountId)`** (optional)  
  Returns schedules: **total**, **released**, **cliff**, **period**.

These read **Creditcoin** runtime storage (via precompile runtime hook), not ERC20 `balanceOf`, for **pending** rewards.

### 5.2 Mutations

- **`claim(uint256 amount)`** or **`claimAll()`**  
  - Resolves **caller identity** (see §6): either EVM-linked attestor or rejected.
  - Updates claimed/vesting state **in Creditcoin** (native runtime).
  - Calls ERC20 **`mint`** to **beneficiary** (caller or fixed linked H160).

- **`claimTo(address beneficiary, uint256 maxAmount)`** (optional)  
  Restricted so only linked keys or governance can redirect (policy).

### 5.3 Optional vesting

- **Vesting** can be implemented **only in Creditcoin’s native runtime** (recommended): accrual increases “earned,” but **claimable** increases over time per schedule. The precompile only mints **claimable** now; the ERC20 stays dumb.

- Alternatively, **token-level vesting** (Solidity) duplicates logic and complicates minter semantics—usually avoid unless required for DeFi composability.

---

## 6. Attestors without EVM keys: linking and claims

### 6.1 Facts

- Attestors authenticate to Creditcoin with **native accounts** (`AccountId`, sr25519/ed25519 as configured).
- Frontier maps **H160 → AccountId** via **`AddressMapping`** (see `pallet_evm` config). There is **no** automatic equality between an attestor’s `AccountId` and “their” H160 unless **explicitly linked**.

### 6.2 Recommended model

1. **Registration extrinsic (Creditcoin native)**  
   `link_evm_beneficiary(evm_address)` signed by the **account that controls rewards**—typically the **stash** when **`Accrued[stash]`** is used (§4.0.1), not each separate attestor identity. Stored in pallet storage, e.g. **`(StashAccountId → H160)`** or **`(RewardAccountId → H160)`**. Optionally allow **one** active link or a cooldown to prevent griefing.

2. **Claim path A — Creditcoin-native first (often simplest)**  
   Extrinsic **`claim_rewards_attest_coin()`**:
   - Signed by the **`AccountId`** that owns **`Accrued`** (usually **stash** per §4.0.1).
   - Reads accrual, applies vesting.
   - Invokes **EVM mint** to the **linked H160** (via `Runner::call` or internal hook), or transfers if using a treasury.

   **Pros**: natural signature type; no ECDSA in precompile for sr25519.  
   **Cons**: UX is Polkadot.js / Creditcoin-compatible wallets, not MetaMask—unless a thin UI wraps both.

3. **Claim path B — EVM transaction**  
   User sends tx from **linked H160**; precompile checks that `AddressMapping::into_account_id(msg.sender)` matches the **registered native AccountId** (or checks a stored link table keyed by hash of `AccountId`).

   **Pros**: MetaMask-friendly.  
   **Cons**: requires **link** in step 1 so `msg.sender` is meaningful.

4. **Claim path C — signature inside precompile (niche)**  
   “Present sr25519 signature over a claim digest” inside an EVM call is **non-standard**, heavy, and toolchain-unfriendly. Prefer **path A or B**.

### 6.3 Security notes

- **Linking** must be **replay-protected** (nonce, chain id, pallet nonce).
- **Changing beneficiary** should be **stash-** (or reward-owner-) signed or time-locked per policy.
- **Precompile `view` methods** can expose accrual **by `AccountId` hash** (stash or attestor id, per §4.0.1); **mutations** must enforce **one** of the claim paths above.

---

## 7. Summary table

| Topic | Recommendation |
|-------|----------------|
| Architecture | **Option A** (§2): ERC-20 as primary liquid token + precompile minter. **Option B** (§3): **native** canonical supply + **wrapped** ERC-20 on EVM, **`wrapped_supply == escrow`**. |
| Vote weight | **Unweighted**; **equal stake** per attestor → equal base credit per eligible work unit; combine with §4.3 for sampling fairness. |
| Token (Option A) | Plain ERC-20; **`MINTER_ROLE` → precompile address**. |
| Token (Option B) | Native asset + **wrapped** precompile or contract; **lock/mint** and **burn/unlock**; no unbacked EVM float. |
| Accrual | Creditcoin runtime **`Accrued`**; **key by stash** when one stash backs multiple attestors (§4.0.1); **cadence** per §4.5. |
| Fairness | §4.0–4.3 + chosen budget model in §4.5. |
| Precompile | **View**: accrued / claimable / vesting; **Mutate**: claim → mint (**A**) or wrap/unwrap leg (**B**). |
| Inner mint (Option A) | Precompile **`CALL`** to ERC-20 if supported; else **`Runner::call`** from pallet. |
| Attestors without an EVM key | **`link_evm_beneficiary`** + **Creditcoin extrinsic claim** and/or **EVM claim** after link; Option B adds **signed wrap** payload (`H160`, amount, nonce). |

---

## 8. Open items (to close before implementation)

- [ ] Choose **Option A** (§2) vs **Option B** (§3): single ERC-20 minter vs native asset + wrapped EVM with escrow invariant.
- [ ] Confirm **`Accrued[stash]`** vs **`Accrued[attestor_id]`** (§4.0.1); link/claim signers must match the chosen key.
- [ ] Confirm **equal-stake** assumption vs any future weighted votes (§4.0).
- [ ] Choose primary **reward cadence**: Babe epoch vs attestation epoch vs staking **era** (§4.5).
- [ ] Pick **Proposal A/B/C** (or hybrid) and exact **`R_period` / `r` / caps** from inflation or treasury policy.
- [ ] Exact **accrual and fairness** formula aligned with attestation sampling (§4.3–4.5).
- [ ] Frontier support for **precompile → ERC20 nested `CALL`** as minter (§2.3–2.4).
- [ ] **Decimals** and **unit** conversion between reward points and ERC20.
- [ ] **Governance** of minter role changes and emergency pause.
- [ ] **Option B** (if selected): escrow account design, **wrap** signature payload (`H160`, amount, nonce), and **unwrap** recipient rules.
- [ ] **Gas and weight** limits for batched claims.
