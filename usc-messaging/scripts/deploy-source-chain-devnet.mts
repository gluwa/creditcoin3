// usc-dev: deploy the CREDITCOIN-side per-chain write-ability stack for a newly registered chain
// key (Outbox, AttestorVault, RelayerFeeVault, RelayerContractLite, AcknowledgmentValidator) and
// wire it into the SHARED contracts already recorded under `source.*` of usc-dev-deploy.json.
//
// Step 3 of 3 when adding a destination chain to usc-devnet (Creditcoin EVM chain id 42):
//   1. register-chain-devnet.mjs        — pallet side, assigns CHAIN_KEY
//   2. deploy-dest-chain-devnet.mts     — destination chain: AttestorRegistry, EOAValidator, Inbox
//   3. deploy-source-chain-devnet.mts   (this)
//
// Shared (reused, from source.*): attest, proofVerifier, deliveryDecoder, factory (OutboxFactory),
// feeRegistry, attestorRegistry, chainRegistry, outboxDiscovery. NOT deployed: legacy
// RelayerContract, TWAPReader, ASCRelayingQuoter (the quoter is off-chain since 2026-09-04).
//
// On-chain calls (Creditcoin, DEPLOYER_KEY = owner of the shared contracts):
//   predict: vault = CREATE(nonce n), feeVault = n+1, lite = n+2,
//            outbox = factory.computeOutboxAddressFor(owner, CHAIN_KEY, owner, owner, 0, vault, feeRegistry, attest)
//   AttestorVault(owner, attest, outbox, lite, owner /*validationContract placeholder*/, attestorRegistry, DEAD, 0)
//   RelayerFeeVault(attest, lite)
//   RelayerContractLite(owner, attest, outbox, proofVerifier, deliveryDecoder)
//   factory.deployOutbox(CHAIN_KEY, owner, owner, 0, vault, feeRegistry, attest)   → OutboxCreated
//   outbox.setTrustedForwarder(lite, true)
//   AcknowledgmentValidator(CHAIN_KEY, owner, proofVerifier, attest)
//   outbox.setValidator(ack); ack.setOutbox(outbox); ack.updateTrustedInbox(destInbox, true)
//   lite.setRelayerFeeVault(feeVault); lite.setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID)
//   lite.setAuthorizedQuoter(QUOTER_EOA, true)                       (only when QUOTER_EOA is set)
//   deliveryDecoder.setTrustedInbox(DEST_CHAIN_ID, destInbox)
//   chainRegistry.setChain(CHAIN_KEY, DEST_CHAIN_ID)
//   outboxDiscovery.registerOutbox(CHAIN_KEY, outbox)                → defaultOutbox(CHAIN_KEY)
// then on the destination chain (DEST_RPC, DEST_ADMIN_KEY = Inbox owner):
//   Inbox.setSupportedOutbox(outbox, true)                           → isSupportedOutbox
// Every wiring step reads its getter first and is skipped when already in place. If
// chains.<CHAIN_KEY>.source.outbox already exists, nothing is redeployed and only the wiring is
// re-checked, so the script can be re-run after a partial failure.
//
// Env:
//   CHAIN_KEY          chain key from register-chain-devnet.mjs                   (required)
//   DEST_CHAIN_ID      destination EVM chain id (default: chains.<CHAIN_KEY>.dest.chainId)
//   DEPLOYER_KEY       Creditcoin deployer / owner of the shared contracts         (required)
//   DEST_RPC           destination-chain JSON-RPC URL                              (required)
//   DEST_ADMIN_KEY     owner of the destination Inbox (chains.<CHAIN_KEY>.dest.admin) (required)
//   QUOTER_EOA         optional; unset = no quoter authorised on the new Lite
//   CC_RPC             default source.rpc / https://rpc.usc-devnet.creditcoin.network
//   ASC_CONTRACTS_DIR  compiled asc-contracts checkout (main c83b3372+)
//   DEPLOY_OUT         default ../usc-dev-deploy.json
//
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

function ascContractsDir(): string {
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+, `npx hardhat compile`)");
  return dir;
}
const UC = ascContractsDir();
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const DEAD = "0x000000000000000000000000000000000000dEaD";
const CC_CHAIN_ID = 42;
const RATE = 0n; // Outbox defaultRateLimit (uint128)

const need = (k: string): string => {
  const v = process.env[k];
  if (!v) throw new Error(`missing ${k}`);
  return v;
};
const CHAIN_KEY = Number(need("CHAIN_KEY"));
if (!Number.isInteger(CHAIN_KEY) || CHAIN_KEY <= 0 || CHAIN_KEY > 0xffff) throw new Error("CHAIN_KEY must be an integer in 1..65535 (ChainRegistry limit)");
const DEPLOYER_KEY = need("DEPLOYER_KEY");
const DEST_RPC = need("DEST_RPC");
const DEST_ADMIN_KEY = need("DEST_ADMIN_KEY");
const QUOTER_EOA = process.env.QUOTER_EOA ? ethers.getAddress(process.env.QUOTER_EOA) : null;

const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
const s = deployJson.source ?? {};
for (const k of ["attest", "proofVerifier", "deliveryDecoder", "factory", "feeRegistry", "attestorRegistry", "chainRegistry", "outboxDiscovery"]) {
  if (!s[k]) throw new Error(`source.${k} missing in ${OUT} — the shared stack must be deployed first (deploy-source-devnet / deploy-discovery-devnet)`);
}
deployJson.chains ??= {};
const entry = deployJson.chains[String(CHAIN_KEY)];
if (!entry?.dest?.inbox) throw new Error(`no chains.${CHAIN_KEY}.dest.inbox in ${OUT} — run deploy-dest-chain-devnet first`);
const destInbox: string = ethers.getAddress(entry.dest.inbox);
const DEST_CHAIN_ID = Number(process.env.DEST_CHAIN_ID ?? entry.dest.chainId ?? entry.destChainId);
if (!Number.isInteger(DEST_CHAIN_ID) || DEST_CHAIN_ID <= 0) throw new Error("DEST_CHAIN_ID unset and not recorded under chains.<CHAIN_KEY>.dest");
if (entry.dest.chainId !== undefined && Number(entry.dest.chainId) !== DEST_CHAIN_ID) {
  throw new Error(`chains.${CHAIN_KEY}.dest.chainId is ${entry.dest.chainId}, but DEST_CHAIN_ID=${DEST_CHAIN_ID}`);
}
const existing = entry.source ?? null;

const rpc = process.env.CC_RPC ?? s.rpc ?? "https://rpc.usc-devnet.creditcoin.network";
const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(DEPLOYER_KEY, provider);
const owner = wallet.address;
const lc = (a: string) => a.toLowerCase();
const same = (a: string, b: string) => lc(a) === lc(b);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}
const at = (addr: string, art: any, signer: ethers.Signer | ethers.Provider = wallet) => new ethers.Contract(addr, art.abi, signer);

const factoryArt = ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory");
const outboxArt = ART("write-ability/Outbox.sol", "Outbox");
const vaultArt = ART("write-ability/AttestorVault.sol", "AttestorVault");
const feeVaultArt = ART("write-ability/RelayerFeeVault.sol", "RelayerFeeVault");
const liteArt = ART("write-ability/RelayerContractLite.sol", "RelayerContractLite");
const ackArt = ART("write-ability/AcknowledgementValidator.sol", "AcknowledgmentValidator"); // file vs contract spelling differ
const decoderArt = ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder");
const chainRegistryArt = ART("write-ability/deployer/ChainRegistry.sol", "ChainRegistry");
const discoveryArt = ART("write-ability/deployer/OutboxDiscovery.sol", "OutboxDiscovery");
const inboxArt = ART("write-ability/Inbox.sol", "Inbox");

const factory = at(s.factory, factoryArt);
const decoder = at(s.deliveryDecoder, decoderArt);
const chainRegistry = at(s.chainRegistry, chainRegistryArt);
const discovery = at(s.outboxDiscovery, discoveryArt);

console.log(`deployer ${owner}  balance ${ethers.formatEther(await provider.getBalance(owner))} CTC  nonce ${await wallet.getNonce()}`);
console.log(`chain key ${CHAIN_KEY} → dest chain ${DEST_CHAIN_ID}, Inbox ${destInbox}${existing ? " (source stack already recorded — wiring only)" : ""}`);

// The shared contracts we mutate must be ours or every setter below reverts.
for (const [name, c] of [["EVMDeliveryDecoder", decoder], ["ChainRegistry", chainRegistry], ["OutboxDiscovery", discovery]] as const) {
  const o: string = await c.owner();
  if (!same(o, owner)) throw new Error(`${name} owner is ${o}, not the deployer ${owner}`);
}
// A different chain key already mapped to this destination chain id would be silently unmapped by setChain.
const priorKey = Number(await chainRegistry.chainKeyOf(DEST_CHAIN_ID));
if (priorKey !== 0 && priorKey !== CHAIN_KEY) throw new Error(`ChainRegistry already maps chain id ${DEST_CHAIN_ID} to chain key ${priorKey}; refusing to steal it for ${CHAIN_KEY}`);

// ---------------------------------------------------------------------------------------------
// 1. Per-chain deploy (skipped when chains.<CHAIN_KEY>.source is recorded).
// ---------------------------------------------------------------------------------------------
let vaultAddr: string, feeVaultAddr: string, liteAddr: string, outboxAddr: string, ackAddr: string;
if (existing) {
  ({ attestorVault: vaultAddr, relayerFeeVault: feeVaultAddr, relayerContract: liteAddr, outbox: outboxAddr, ackValidator: ackAddr } = existing);
  for (const [k, v] of Object.entries({ vaultAddr, feeVaultAddr, liteAddr, outboxAddr, ackAddr })) if (!v) throw new Error(`chains.${CHAIN_KEY}.source is incomplete (${k}); delete it to redeploy`);
  if ((existing.relayerContractKind ?? "RelayerContractLite") !== "RelayerContractLite") throw new Error(`chains.${CHAIN_KEY}.source.relayerContractKind is ${existing.relayerContractKind}; this script only wires RelayerContractLite`);
} else {
  // vault (nonce n), feeVault (n+1), lite (n+2); Outbox via CREATE2 — resolves the ctor circularity.
  const n = await wallet.getNonce();
  const predict = (k: number) => ethers.getCreateAddress({ from: owner, nonce: n + k });
  [vaultAddr, feeVaultAddr, liteAddr] = [predict(0), predict(1), predict(2)];
  outboxAddr = await factory.computeOutboxAddressFor(owner, CHAIN_KEY, owner, owner, RATE, vaultAddr, s.feeRegistry, s.attest);
  if ((await provider.getCode(outboxAddr)) !== "0x") {
    throw new Error(`predicted Outbox ${outboxAddr} already has code (same salt inputs deployed before) but chains.${CHAIN_KEY}.source is not recorded; record it by hand or change the inputs`);
  }
  const curDefault: string = await discovery.defaultOutbox(CHAIN_KEY);
  if (curDefault !== ethers.ZeroAddress) throw new Error(`OutboxDiscovery.defaultOutbox(${CHAIN_KEY}) is already ${curDefault}; a rotation needs setDefaultOutbox + timelock, not this script`);
  console.log(`  predicted: vault ${vaultAddr}  feeVault ${feeVaultAddr}  lite ${liteAddr}  outbox ${outboxAddr}`);

  const vault = await deploy("AttestorVault", vaultArt, [owner, s.attest, outboxAddr, liteAddr, owner, s.attestorRegistry, DEAD, 0]);
  const feeVault = await deploy("RelayerFeeVault", feeVaultArt, [s.attest, liteAddr]);
  const lite = await deploy("RelayerContractLite", liteArt, [owner, s.attest, outboxAddr, s.proofVerifier, s.deliveryDecoder]);
  for (const [name, c, want] of [["AttestorVault", vault, vaultAddr], ["RelayerFeeVault", feeVault, feeVaultAddr], ["RelayerContractLite", lite, liteAddr]] as const) {
    if (!same(await c.getAddress(), want)) throw new Error(`${name} precompute mismatch: got ${await c.getAddress()}, predicted ${want}`);
  }

  console.log("  deployOutbox via factory…");
  const rcpt = await (await factory.deployOutbox(CHAIN_KEY, owner, owner, RATE, vaultAddr, s.feeRegistry, s.attest)).wait();
  const created = rcpt!.logs.map((l: any) => { try { return factory.interface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "OutboxCreated");
  if (!created) throw new Error("deployOutbox receipt has no OutboxCreated event");
  if (!same(created.args.outbox, outboxAddr)) throw new Error(`Outbox CREATE2 mismatch: event ${created.args.outbox}, predicted ${outboxAddr}`);
  console.log(`  Outbox → ${outboxAddr} (chainKey ${created.args.chainKey}, version ${created.args.version})`);

  const ack = await deploy("AcknowledgmentValidator", ackArt, [BigInt(CHAIN_KEY) /* uint64 destinationChainKey */, owner, s.proofVerifier, s.attest]);
  ackAddr = await ack.getAddress();

  // Record immediately so a wiring failure below can be resumed instead of redeploying.
  deployJson.chains[String(CHAIN_KEY)] = {
    ...entry,
    source: {
      outbox: outboxAddr, attestorVault: vaultAddr, relayerFeeVault: feeVaultAddr,
      relayerContract: liteAddr, relayerContractKind: "RelayerContractLite", ackValidator: ackAddr,
      quoterEOA: QUOTER_EOA, deployedAt: new Date().toISOString(),
    },
  };
  writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
}

const outbox = at(outboxAddr, outboxArt);
const lite = at(liteAddr, liteArt);
const ack = at(ackAddr, ackArt);
if (Number(await outbox.chainKey()) !== CHAIN_KEY) throw new Error(`Outbox.chainKey() is ${await outbox.chainKey()}, expected ${CHAIN_KEY}`);
if (Number(await ack.destinationChainKey()) !== CHAIN_KEY) throw new Error("AcknowledgmentValidator.destinationChainKey() mismatch");

// ---------------------------------------------------------------------------------------------
// 2. Wiring on Creditcoin — each step: read getter, act only if needed.
// ---------------------------------------------------------------------------------------------
async function ensure(label: string, isDone: () => Promise<boolean>, act: () => Promise<ethers.ContractTransactionResponse>) {
  if (await isDone()) { console.log(`  ✓ ${label} (already)`); return; }
  await (await act()).wait();
  if (!(await isDone())) throw new Error(`${label} landed but the getter still disagrees`);
  console.log(`  ✓ ${label}`);
}

await ensure(`Outbox.setTrustedForwarder(${liteAddr}, true)`,
  () => outbox.isTrustedForwarder(liteAddr), () => outbox.setTrustedForwarder(liteAddr, true));
await ensure(`Outbox.setValidator(${ackAddr})`,
  async () => same(await outbox.validator(), ackAddr), () => outbox.setValidator(ackAddr));
await ensure(`AcknowledgmentValidator.setOutbox(${outboxAddr})`,
  async () => same(await ack.outbox(), outboxAddr), () => ack.setOutbox(outboxAddr));
await ensure(`AcknowledgmentValidator.updateTrustedInbox(${destInbox}, true)`,
  () => ack.trustedInboxes(destInbox), () => ack.updateTrustedInbox(destInbox, true));
await ensure(`RelayerContractLite.setRelayerFeeVault(${feeVaultAddr})`,
  async () => same(await lite.relayerFeeVault(), feeVaultAddr), () => lite.setRelayerFeeVault(feeVaultAddr));
await ensure(`RelayerContractLite.setDestinationEvmChainId(${CHAIN_KEY}, ${DEST_CHAIN_ID})`,
  async () => Number(await lite.destinationEvmChainIds(CHAIN_KEY)) === DEST_CHAIN_ID, () => lite.setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID));
if (QUOTER_EOA) {
  await ensure(`RelayerContractLite.setAuthorizedQuoter(${QUOTER_EOA}, true)`,
    () => lite.authorizedQuoters(QUOTER_EOA), () => lite.setAuthorizedQuoter(QUOTER_EOA, true));
} else console.log("  - QUOTER_EOA unset: no quoter authorised on the new RelayerContractLite (use add-quoter.mts later)");
await ensure(`EVMDeliveryDecoder.setTrustedInbox(${DEST_CHAIN_ID}, ${destInbox})`,
  async () => same(await decoder.trustedInboxes(DEST_CHAIN_ID), destInbox), () => decoder.setTrustedInbox(DEST_CHAIN_ID, destInbox));
// registerOutbox reverts UnknownChainKey unless the ChainRegistry mapping exists, so setChain goes first.
await ensure(`ChainRegistry.setChain(${CHAIN_KEY}, ${DEST_CHAIN_ID})`,
  async () => Number(await chainRegistry.chainIdOf(CHAIN_KEY)) === DEST_CHAIN_ID, () => chainRegistry.setChain(CHAIN_KEY, DEST_CHAIN_ID));
{
  const cur: string = await discovery.defaultOutbox(CHAIN_KEY);
  if (cur !== ethers.ZeroAddress && !same(cur, outboxAddr)) throw new Error(`OutboxDiscovery.defaultOutbox(${CHAIN_KEY}) is ${cur}, not our ${outboxAddr}; rotation needs setDefaultOutbox + timelock`);
  // First registration for a key becomes the default immediately (no timelock).
  await ensure(`OutboxDiscovery.registerOutbox(${CHAIN_KEY}, ${outboxAddr})`,
    async () => same(await discovery.defaultOutbox(CHAIN_KEY), outboxAddr), () => discovery.registerOutbox(CHAIN_KEY, outboxAddr));
}

// ---------------------------------------------------------------------------------------------
// 3. Destination chain: allowlist the Outbox on the Inbox (or the relayer's first delivery reverts
//    UnsupportedOutbox).
// ---------------------------------------------------------------------------------------------
{
  const destProvider = new ethers.JsonRpcProvider(DEST_RPC, DEST_CHAIN_ID, { staticNetwork: true });
  const liveChainId = Number(BigInt(await destProvider.send("eth_chainId", [])));
  if (liveChainId !== DEST_CHAIN_ID) throw new Error(`DEST_RPC reports chain id ${liveChainId}, expected ${DEST_CHAIN_ID}`);
  const destAdmin = new ethers.Wallet(DEST_ADMIN_KEY, destProvider);
  const inbox = at(destInbox, inboxArt, destAdmin);
  const inboxOwner: string = await inbox.owner();
  if (!same(inboxOwner, destAdmin.address)) throw new Error(`Inbox ${destInbox} owner is ${inboxOwner}, but DEST_ADMIN_KEY is ${destAdmin.address}`);
  if ((await inbox.localChainKey()).toLowerCase() !== ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32).toLowerCase()) throw new Error(`Inbox.localChainKey() is not bytes32(${CHAIN_KEY})`);
  console.log(`dest admin ${destAdmin.address}  balance ${ethers.formatEther(await destProvider.getBalance(destAdmin.address))} native`);
  await ensure(`Inbox.setSupportedOutbox(${outboxAddr}, true) on chain ${DEST_CHAIN_ID}`,
    () => inbox.isSupportedOutbox(outboxAddr), () => inbox.setSupportedOutbox(outboxAddr, true));
  destProvider.destroy();
}

deployJson.chains[String(CHAIN_KEY)] = {
  ...deployJson.chains[String(CHAIN_KEY)],
  source: {
    ...(deployJson.chains[String(CHAIN_KEY)].source ?? {}),
    outbox: outboxAddr, attestorVault: vaultAddr, relayerFeeVault: feeVaultAddr,
    relayerContract: liteAddr, relayerContractKind: "RelayerContractLite", ackValidator: ackAddr,
    quoterEOA: QUOTER_EOA ?? existing?.quoterEOA ?? null,
    deployedAt: existing?.deployedAt ?? deployJson.chains[String(CHAIN_KEY)].source?.deployedAt ?? new Date().toISOString(),
    wiredAt: new Date().toISOString(),
  },
};
writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
console.log(`✅ chain key ${CHAIN_KEY} source stack ready. Outbox ${outboxAddr}  RelayerContractLite ${liteAddr}  ack ${ackAddr}`);
console.log(`   recorded chains.${CHAIN_KEY}.source in ${OUT}. Next: point attestors/relayer at chain key ${CHAIN_KEY} and replace the placeholder attestor set on the destination registry.`);
process.exit(0);
