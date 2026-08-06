# USC Write-Ability — usc-dev Deploy & Wire Runbook (DRAFT)

Target: first live deployment of the write-ability pipeline on **cc3-devnet** (usc-dev), route
`Creditcoin devnet EVM (chain id 42 on cc3-usc-dev) → Sepolia (chain_key 8)`.

Ordering is load-bearing: contracts before pallet wiring (the extrinsics take addresses), pallet
wiring before attestors (they idle until `WriteAbilityConfigs` is set — by design), attestors
before the relayer (it aggregates their votes).

## 0. Version matrix (pin these, no mutable tags)

| Component | Source | Version to deploy |
|---|---|---|
| creditcoin3 node/runtime | gluwa/creditcoin3 usc-dev | first usc-dev build **after #1215 merges** (spec 131 + `supportedChains` V2 migrations) |
| attestor | same build as above | same image tag as the node |
| relayer | gluwa/usc-message-relayer | **`0.1.1`** Docker tag (v0.1.0 has the `RELAYER_RELAYER_FEE_VAULT_ADDRESS` env gap) |
| contracts | gluwa/usc-contracts | `main` ≥ `1f8b553` (post-#23) |
| proof-gen | creditcoin3 build | already running on cc3-devnet (bridge claim path) — reuse, verify it serves chain_key 8 |
| cc3-indexer | creditcoin3 `cli/` | post-#1215 (Outbox discovery handlers + `canAck` schema) |
| quoter (usc-messaging) | creditcoin3 `usc-messaging/` | ⚠️ **blocked** — see §7 |

## 1. Runtime upgrade (Creditcoin devnet)

1. Merge #1215 → usc-dev release build (srtool WASM).
2. Upgrade devnet runtime the usual way (sudo `system.setCode` on devnet).
3. Verify: `supportedChains` palletVersion == **2**, `CoreFees` empty,
   `chainInfo`/`0x…0FD3` answers `get_core_fee(uint32)` (selector `0x5b023376`) with 0 for chain 8
   (unset ⇒ 0 until §3.3).

Rollback: runtime upgrades roll forward only; the migration only clears an empty map, so there is
no data-loss risk. If the upgrade must be abandoned, re-set the previous WASM.

## 2. Contract deployment

Reference implementation: `usc-messaging/scripts/deploy-dest-ethers.mts` + `deploy-source-ethers.mts`
(what the e2e runs against anvil — same sequence, real endpoints). Addresses below land in a
deploy JSON; keep it, everything downstream consumes it.

### 2a. Destination chain (Sepolia)

1. **EOAValidator** — constructor takes the initial attestor set + threshold. Bootstrap with the
   devnet attestors' write-ability EVM addresses if already known, else deploy with the operator
   as a placeholder set and let the set-update flow replace it (§4.3).
2. **Inbox** — wired to the EOAValidator.
3. **Test consumer dApp** (MockDestination equivalent) — for the §8 smoke test.

### 2b. Source chain (Creditcoin devnet EVM)

1. **FeeRegistry** — point its core-fee provider at the **chain-info precompile**
   (`ICoreFeeProvider` = `0x…0FD3`); register the quoter EOA address.
2. **OutboxFactory** (CREATE2) → create the chain-8 **Outbox** through it (the 5-arg
   `OutboxCreated` event is what the attestor resolver and the indexer both scan for — do NOT
   deploy an Outbox directly).
3. **RelayerContract** (fee ledger: `publishAndCollectRelayerFee`, `claimDelivery`,
   `withdrawNative`).
4. **AcknowledgmentValidator** — wired as the Outbox's ack validator; verifies native USC proofs
   from proof-gen.
5. Post-deploy wiring: `Inbox.updateTrustedInbox`-style cross-links per the deploy scripts
   (outbox↔inbox trust, trusted forwarders, `FeeRegistry` quoter registration). Follow
   `deploy-source-ethers.mts` step-for-step.

⚠️ The standalone `usc-messaging/scripts/deploy.ts` is **legacy pre-#23** (2-arg `OutboxCreated`)
— do not use it. Use the `-ethers.mts` pair with `USC_CONTRACTS_DIR` pointing at a post-#23
checkout, until Kevin publishes the post-#23 `@gluwa/usc-contracts` npm package.

## 3. Pallet wiring (sudo on devnet, in this order)

```
1. sudo(supportedChains.setOutboxFactoryAddr(8, <factory addr from 2b.2>))
2. sudo(supportedChains.setWriteAbilityConfig(8, { enabled: true, ... }))
3. sudo(supportedChains.setCoreFee(8, <amount in ATTEST wei>))
```

`register-factory.mjs` does step 1 programmatically. Until step 2, attestors stay silently idle;
until step 3, `get_core_fee` returns 0 and the FeeRegistry floor is core-fee-free — fine for the
first smoke test, set a real value before opening to users.

## 4. Attestor rollout

1. Roll the devnet attestor StatefulSet to the post-#1215 image (same tag as the node). IaC:
   `cc-networks-iac/k8s`, branch `usc/messaging`.
2. Per-attestor write-ability env/flags: destination RPC (Sepolia — give each a **different
   provider** where possible; single-LB skew is the known `BlockHeaderRootsMismatch` cause),
   EOAValidator address, write-ability signer key (secret). On boot each attestor registers its
   EVM address on-chain (`set_attestor_evm_address`) and re-checks every proposer tick.
3. **Set sync**: once > 2/3 of the active committee is registered, attestors gossip signed
   set-update votes and the relayer submits `submitAttestorSetUpdate` — this replaces any
   bootstrap set from §2a.1. Watch for `proposing attestor-set update` then
   `EOAValidator attestor set updated`.
4. Verify: logs show Outbox resolved via chain-info → factory scan, and `MessagePublished`
   subscription active.

## 5. Relayer deploy

IaC gaps to close first (tracked): pin chart image to `gluwa/usc-message-relayer:0.1.1`, add
liveness/readiness probes (`GET /health` on 3200), Secret template for signer keys.

Config (`config.example.yaml` in the relayer repo is the annotated reference):

- `routes[0]`: `chain_key: 8`, `creditcoin_chain_id: 102035`, `outbox_address` (§2b.2),
  `destination_rpc_url` (https Sepolia), `inbox_address` (§2a.2), `signer_key` (funded Sepolia
  key), `block_confirmation_depth: 0` (GRANDPA).
- `attestor_set`: `kind: evm_contract`, `address:` EOAValidator (§2a.1) — hot-reloads, no restart
  on set changes.
- `relayer_contract_address`: RelayerContract (§2b.3) — enables funded-gas delivery +
  `claimDelivery`.
- `ack:` block — **required** whenever `relayer_contract_address` is set, or relay fees are never
  claimed: `proof_gen_url` (devnet proof-gen), `validator_address` (§2b.4), funded Creditcoin
  `signer_key`, `confirmation_depth: 64` (Sepolia).
- Vote source: embedded `p2p` with the devnet attestor bootnode multiaddrs (spy-node mode is the
  target architecture but optional for first deploy).

Fund both signers before start; the relayer pays destination gas out of pocket and recoups via
`claimDelivery`.

## 6. Indexer

Deploy the post-#1215 cc3-indexer (new handlers + `canAck` schema change ⇒ schema migration /
reindex of the Outbox entities). It discovers Outboxes chain-wide by `OutboxCreated` topic — no
per-deployment config needed.

## 7. Quoter — the one blocked component

The `usc-messaging` quoter still signs the **pre-#23 quote preimage**; post-#23 `FeeRegistry`
expects the v3 struct (`acknowledgmentPrice` restored, `requiresAck` dropped, `payInNative`).
Publishing through the fee path fails signature validation until the quoter is fixed.

- Fix is in our repo (small): update the EIP-712 struct in `usc-messaging/src/quoter/index.ts` to
  the v3 preimage. Not gated on the npm publish (that gates only the deploy scripts/ABIs).
- Until fixed: §8's smoke test can still exercise publish→deliver→ack by quoting manually
  (operator-signed quote via a script against the post-#23 artifacts), but user-facing publish is
  down.

## 8. Smoke test (definition of done)

Mirror the e2e assertions, on devnet:

1. Quote → approve ATTEST → `RelayerContract.publishAndCollectRelayerFee` (or 2-arg
   `Outbox.publishMessage` via the quote) with `canAck: true`.
2. Attestors: ≥ threshold `MessagePublished` votes gossiped (logs).
3. Destination: consumer dApp state changed; `MessageDelivered` on the Inbox.
4. Ack: `submitAcknowledgment` lands on the AcknowledgmentValidator; `Outbox` marks the message
   acknowledged (indexer `canAck`/acknowledged fields flip).
5. Claim: `claimDelivery` succeeds; relayer ledger balance withdrawn-able; no
   `NativeTransferFailed` retries stuck in the relayer log.
6. Grafana: attestor + relayer dashboards clean for 1h (no `BlockHeaderRootsMismatch`, no
   terminal-revert loops).

## 9. Rollback / kill switches

- Pallet: `setWriteAbilityConfig(8, enabled: false)` — attestors stop attesting messages; publish
  keeps working on the EVM side but nothing is delivered.
- Contracts: Outbox/Inbox are `Pausable` — pause on the destination stops delivery instantly.
- Relayer: scale to 0; scan cursors + `scan_lookback_blocks: 600` re-discover in-flight work on
  restart, settlements are idempotent (`RelayAlreadySettled` dedup).

## Open items before executing

1. Merge #1215 (CI + review gate).
2. Relayer v0.1.1 image published (release in flight).
3. Quoter v3 preimage fix (§7) — the only code change left.
4. IaC: relayer chart pin/probes/Secret, quoter chart (0% today), attestor env additions.
5. Kevin: post-#23 npm publish (unblocks clean deploy scripts; manual `USC_CONTRACTS_DIR` path
   works meanwhile).
