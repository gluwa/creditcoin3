# Write-ability network upgrade runbook (usc-devnet, 10 Sep 2026)

The complete, ordered procedure that took usc-devnet from runtime 131 / #1322-less attestors to
runtime 136, the Base-branch attestor image on the whole fleet, asc-contracts #36 Inboxes with
DispatcherRouter on two destinations (Sepolia, Base Sepolia), relayer `main-a7531b7` with two
routes, proof-gen for both chains, and a working publish → deliver → ack → claim loop on both.
Written to be executed again for the next environment (cc3-devnet, usc-testnet) or the next
destination chain. Every script lives in `usc-messaging/scripts/`; all keys used are DEVNET-ONLY.

Total wall time on usc-devnet: ~4 h including the two detours in §10.

## 0. Preconditions and inputs

| Item | usc-devnet value | Notes |
| --- | --- | --- |
| Sudo | `//Alice` | `SUDO_URI`; `runtime-upgrade-devnet.mjs` refuses a non-devnet RPC and a sudo mismatch |
| Creditcoin deployer | `0xf24FF3a9…` (`~/.usc-dev-deployer.env`, `DEPLOYER_KEY`) | owns Outboxes, vaults, decoder, ack validators, new Lites |
| Quoter EOA | `0x7CC15788…` (`~/.usc-dev-quoter.env`) | never the deployer |
| Destination admin / relayer signer | Sepolia `0x66C27cfd…` = relayer `route-0-signer-key`; Base `0x97c2cdaB…` (`~/.usc-dev-base.env`) | needs gas on the destination |
| Compiled contracts | `ASC_CONTRACTS_DIR` = asc-contracts main checkout, `npx hardhat compile` | main a9791c37 (#36) |
| Attestor image | `gluwa/creditcoin3-next:writeability-<sha9>` | CI does NOT push it: download the Docker workflow `locally-built-docker-image` artifact, `docker load`, retag, push |
| Relayer image | `gluwa/asc-message-relayer:main-<sha7>` | pushed by relayer CI |
| Operator | `gluwa/attestor-operator` ≥ 0.5.3 | needed for `spec.ethChainFamily` (OP-Stack) |
| IaC | cc-networks-iac `k8s/networks/cc3-usc-dev/cc3-usc-dev/**`, branch `usc/messaging` | attestors are operator `AttestorSet` CRs, the rest is Helm |
| Clusters | `usc-devnet-cluster` (relayer, spy, proof-gen, Base set, 4 Sepolia attestors), `usc-devnet-kc-cluster`, `usc-devnet-we-cluster` | namespace `creditcoin`; `cc3-dryrun-devnet-cluster` is a different network |

Decide up front, per destination chain: EVM chain id, attestation interval, checkpoint interval,
target sample size (quorum is `2/3·target + 1` and target is a cap: keep it ≤ fleet size but ≥ 2),
max attestors, genesis block (a recent multiple of the interval), maturity (`FixedDelay: 20` on
OP-Stack until #1330 lands), core fee, chain family (`ethereum` | `op-stack`).

## 1. Runtime

1. Check what the live chain is built with: `babe.epochDuration` = 15 blocks ⇒ **fast-runtime**.
   The CI srtool wasm (default features, 2 h epochs) would change BABE's epoch length and stall the
   chain. Build locally from the branch that is going out:
   `cargo build --release -p creditcoin3-runtime --features fast-runtime` (add `try-runtime` for the
   check build). `subwasm info` → spec, tx version, blake2.
2. try-runtime against live state:
   `try-runtime --runtime <try.wasm> on-runtime-upgrade --blocktime 5000 --checks pre-and-post live --uri wss://rpc.usc-devnet.creditcoin.network`.
   The migration and idempotency sections are the verdict; the later empty-block panic
   "Timestamp slot must match CurrentSlot" is a try-runtime artefact. Known false red:
   `pallet_supported_chains::migrations::MigrateV1ToV2::post_upgrade` asserts empty core fees even
   when skipped.
3. Upgrade: `WASM=<fast.wasm> EXPECT_SPEC_VERSION=<n> node scripts/runtime-upgrade-devnet.mjs --dry-run`,
   then with `SUDO_URI` and without `--dry-run` (`sudo.sudoUncheckedWeight(system.setCode)`; waits
   for `CodeUpdated`, polls the spec). Watch finality for ten minutes.

## 2. On-chain messaging config (sudo + deployer)

1. Registry-only Outbox discovery (needs the #1292 runtime getter):
   `scripts/deploy-discovery-devnet.mts` (ChainRegistry, OutboxDiscovery proxy, `registerOutbox`) once,
   then `ONLY_DISCOVERY=true CHAIN_KEY=<K> SUDO_URI=… node scripts/pallet-config-devnet.mjs` per chain
   (`set_outbox_discovery_addr`). Verify with the chain-info precompile
   `get_outbox_discovery_address(K)`.
2. New destination chain: `SUDO_URI=… node scripts/register-chain-devnet.mjs` (chain name map key is
   a **string**, not `Uint8Array`), then `deploy-dest-chain-devnet.mts` on the destination
   (EOAValidator, AttestorRegistry, DefaultDispatcher, DispatcherRouter, Inbox with the router,
   MockDestination) and `deploy-source-chain-devnet.mts` on Creditcoin (Outbox, AttestorVault,
   RelayerFeeVault, RelayerContractLite, AcknowledgmentValidator, `setAuthorizedQuoter`,
   `decoder.setTrustedInbox`). Both are resumable and getter-guarded.
3. Existing destination whose Inbox ABI changed: `deploy-dest-devnet.mts` (router stack + new Inbox
   with `INITIAL_OUTBOXES`), then `NEW_INBOX=… node scripts/repoint-inbox-devnet.mts`
   (`AcknowledgmentValidator.updateTrustedInbox`, `EVMDeliveryDecoder.setTrustedInbox`;
   `REVOKE_OLD=true` later).
4. **Decoder.** The `deliverMessage` selector is baked into `EVMDeliveryDecoder`'s bytecode. Whenever
   the Inbox ABI changes: `node scripts/redeploy-decoder-devnet.mts` (deploys from
   `ASC_CONTRACTS_DIR`, trusts every route's Inbox, `setDeliveryDecoder` on every deployer-owned
   Lite). Lites owned by someone else need that owner to call `setDeliveryDecoder(<new>)`.
5. Destination AttestorRegistry bootstrap: the owner must `updateAttestorSet([attestor EVM
   signers])` once; attestor-gossiped set updates validate against the current (placeholder) set
   and never pass otherwise. Derive signers with `scripts/derive-attestor-evm.mjs`.

## 3. Operator and CRD (only if the CRD schema changed)

`kubectl apply -f crd/attestorset-crd.yaml` **before** any CR uses the new field (the schema prunes
unknown fields), then `kubectl set image deploy/cc3-usc-dev-attestor-operator …:<ver> -n creditcoin`.
Operator image: `docker buildx --builder attest` (docker-container driver, needed for
`--sbom/--provenance`).

## 4. Attestors

1. Image: download the Docker workflow artifact for the branch head, `docker load`, retag to
   `gluwa/creditcoin3-next:writeability-<sha9>`, push. Two `docker load`s of `gluwa/creditcoin3:latest`
   need an immediate retag between them.
2. New chain: one `AttestorSet` CR per cluster (`chainKey`, `ethChainFamily`, distinct p2p/api
   ports, `eth-url` secret), archiver (`startHeight` = genesis, `finalizationLag`, `flushEvery: 1`,
   `ETH_CHAIN_FAMILY` via `extraEnv`).
3. Roll: bump `spec.image.tag` in every `attestorset.yaml`, `kubectl apply` **one cluster at a
   time** (Parallel pod policy), wait for the `write-ability activated — Outbox resolved`,
   listener and voted markers before the next cluster.
4. Proof-gen: bump the image, add the chain (`ethChainFamily`, archiver service, depth). Config-only
   Helm changes need `kubectl rollout restart sts/cc3-usc-dev-proof-gen-api`.

## 5. Relayer and spy node (one deployment each, multi-chain)

1. Spy node: `chainKeys: [8, 9, …]`, one bootnode multiaddr per chain
   (`/dns4/cc3-usc-dev-<chain>-attestor-bootnode-headless/tcp/<p2p>/p2p/<peerId>`), then
   `rollout restart` (config-only).
2. Relayer: image `main-<sha7>`, one route per chain (`chainKey`, `inboxAddress`, destination RPC,
   signer key, attestor-set contract, ack validator, Lite, depth), votes from the spy node.
   Route options: `max_native_coin_value_wei` (default 0), `max_gas_limit` (5M),
   `auto_request_top_up` (off). Restart rewinds scans by 600 blocks; decoder reverts are classified
   terminal and never retried (use `claim-delivery-devnet.mts`).
3. Commit the IaC to `usc/messaging` via ADO PRs (squash, "title (PR N)").

## 6. Prove it, per chain

```sh
CHAIN_KEY=<K> npx tsx scripts/publish-lite-devnet.mts            # envelope payload, quote from the quoter EOA
# relayer log: "message delivered chain_key=K … signer_count=N"
# then "submitAcknowledgment confirmed" and "delivery settled on source chain"
CHAIN_KEY=<K> MESSAGE_ID=… DELIVERY_TX=… DRY_RUN=true npx tsx scripts/claim-delivery-devnet.mts   # if the relayer gave up
```

Check on chain: `Outbox.isAcknowledged(messageId)`, `Lite.getMessageInfo(messageId).relaySettled`.

## 7. Record

`usc-dev-deploy.json` (source, dest, `chains.<K>`, `old*` fields), memory/runbook artifacts, and the
IaC PRs. Keep old Inboxes trusted on the ack validator until nothing pends on them, then
`REVOKE_OLD=true`.

## 8. Rollback levers

- Runtime: `setCode` back to the previous wasm (keep the old wasm and its blake2).
- Attestors: previous `spec.image.tag`, one cluster at a time.
- Relayer route: `inboxAddress` back to the previous Inbox; `repoint-inbox-devnet.mts` with the old
  Inbox; `redeploy-decoder-devnet.mts NEW_DECODER=<old>`.
- The Inbox ↔ dispatcher contract pairs are immutable; roll forward with a new pair.

## 9. Order used on 10 Sep (for reference)

runtime (fast-runtime 136) → discovery for 8 and 9 → operator 0.5.3 + CRD → register Base (chain
key 9) → Base destination + source contracts → Base attestor set + archiver → proof-gen (8 + 9) →
Sepolia attestors rolled (3 clusters) → relayer two routes + spy node → Base publish/deliver/ack →
Sepolia #36 Inbox + router (detour) → decoder redeploy (detour) → Base claim.

## 10. Detours worth remembering

- The c83b3372 (#48) Inbox calls `IMessageReceiver.receiveMessage`; the #36 router expects
  `IMessageDispatcher.deliverMessage`. Deliveries park pending forever on the older Inbox. Deploy
  Inbox + router from the same commit.
- The 6 Aug decoder hard-coded the 4-arg `deliverMessage`; claims reverted
  `UnsupportedDestinationTransaction` on both routes (asc-contracts #49, closed as mis-filed).
- polkadot-js: a `Uint8Array` map key is read as SCALE `Bytes` ("Bytes length … exceeds 10485760").
- Base 2 s blocks: `waitForDeployment()` before deploying dependants.
- `ensureDestinationTrusts` must tolerate destinations without `isTrustedInbox`.
