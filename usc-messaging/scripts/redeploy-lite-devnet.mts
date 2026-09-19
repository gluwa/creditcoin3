// Redeploy the RelayerContractLite (+ its RelayerFeeVault) for an EXISTING source-chain stack on
// usc-devnet, owned by the deployer, and wire it to the existing Outbox, proof verifier, decoder.
//
// Why: the Sepolia (chain key 8) Lite 0x6bEC0843… is owned by a third-party EOA we cannot reach, and
// it still points at the pre-#48 decoder, so relay-fee claims revert. Everything else on the route
// (Outbox, AttestorVault, ack validator, Inbox) is unaffected: the Lite only fronts fee collection
// and payout, so a fresh one can take over. Fees already deposited on the old Lite stay in its vault.
//
// Sequence (nonce-predicted, same pattern as deploy-source-chain-devnet):
//   RelayerFeeVault(attest, predictedLite)                           nonce n
//   RelayerContractLite(owner, attest, outbox, proofVerifier, decoder) nonce n+1
//   outbox.setTrustedForwarder(lite, true)   (Outbox owner = deployer)
//   lite.setRelayerFeeVault(feeVault); lite.setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID)
//   lite.setAuthorizedQuoter(QUOTER_EOA, true)
//   decoder.setTrustedInbox(DEST_CHAIN_ID, destInbox)  (idempotent)
// Then: relayer IaC route relayerContractAddress -> new Lite; every emitter re-runs
// Outbox.approveForwarder(newLite, true) (publish-lite-devnet.mts does this itself).
//
// Env: DEPLOYER_KEY, ASC_CONTRACTS_DIR, QUOTER_EOA (dedicated quoter, never the deployer),
//      optional CHAIN_KEY (default 8), CC_RPC, DEPLOY_OUT, DRY_RUN=true (predict only).
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
if (!UC) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+)");
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const CC_CHAIN_ID = 42;
const need = (k: string): string => { const v = process.env[k]; if (!v) throw new Error(`missing ${k}`); return v; };
const CHAIN_KEY = Number(process.env.CHAIN_KEY ?? 8);
const DEPLOYER_KEY = need("DEPLOYER_KEY");
const QUOTER_EOA = ethers.getAddress(need("QUOTER_EOA"));
const DRY_RUN = (process.env.DRY_RUN ?? "false") === "true";

const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
// Chain key 8 lives in source/dest; other chains under chains.<K>.
const src = CHAIN_KEY === 8 ? deployJson.source : deployJson.chains?.[String(CHAIN_KEY)]?.source;
const dst = CHAIN_KEY === 8 ? deployJson.dest : deployJson.chains?.[String(CHAIN_KEY)]?.dest;
if (!src?.outbox || !dst?.inbox) throw new Error(`no source.outbox / dest.inbox recorded for chain key ${CHAIN_KEY} in ${OUT}`);
const DEST_CHAIN_ID = Number(process.env.DEST_CHAIN_ID ?? dst.chainId ?? (CHAIN_KEY === 8 ? 11155111 : undefined));
if (!Number.isInteger(DEST_CHAIN_ID) || DEST_CHAIN_ID <= 0) throw new Error("DEST_CHAIN_ID unknown");
const shared = deployJson.source;
for (const k of ["attest", "proofVerifier", "deliveryDecoder"]) if (!shared[k]) throw new Error(`source.${k} missing`);
if (QUOTER_EOA.toLowerCase() === new ethers.Wallet(DEPLOYER_KEY).address.toLowerCase()) throw new Error("QUOTER_EOA must not be the deployer");

const provider = new ethers.JsonRpcProvider(process.env.CC_RPC ?? shared.rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(DEPLOYER_KEY, provider);
const owner = wallet.address;
const lc = (a: string) => a.toLowerCase();
const same = (a: string, b: string) => lc(a) === lc(b);
const at = (addr: string, art: any) => new ethers.Contract(addr, art.abi, wallet);

const feeVaultArt = ART("write-ability/RelayerFeeVault.sol", "RelayerFeeVault");
const liteArt = ART("write-ability/RelayerContractLite.sol", "RelayerContractLite");
const outboxArt = ART("write-ability/Outbox.sol", "Outbox");
const decoderArt = ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder");

const outbox = at(src.outbox, outboxArt);
const decoder = at(shared.deliveryDecoder, decoderArt);
console.log(`chain key ${CHAIN_KEY} → EVM ${DEST_CHAIN_ID}; deployer ${owner} balance ${ethers.formatEther(await provider.getBalance(owner))} CTC`);
console.log(`  outbox ${src.outbox} (owner ${await outbox.owner()})  decoder ${shared.deliveryDecoder}  old Lite ${src.relayerContract}`);
if (!same(await outbox.owner(), owner)) throw new Error("Outbox owner is not the deployer; cannot trust a new forwarder");

const n = await wallet.getNonce();
const predictedFeeVault = ethers.getCreateAddress({ from: owner, nonce: n });
const predictedLite = ethers.getCreateAddress({ from: owner, nonce: n + 1 });
console.log(`  predicted: feeVault ${predictedFeeVault}  lite ${predictedLite}`);
if (DRY_RUN) { console.log("DRY_RUN — stopping before any transaction"); process.exit(0); }

async function deploy(name: string, art: any, args: any[]) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}
const feeVault = await deploy("RelayerFeeVault", feeVaultArt, [shared.attest, predictedLite]);
const lite = await deploy("RelayerContractLite", liteArt, [owner, shared.attest, src.outbox, shared.proofVerifier, shared.deliveryDecoder]);
if (!same(await feeVault.getAddress(), predictedFeeVault) || !same(await lite.getAddress(), predictedLite)) {
  throw new Error("nonce prediction mismatch — the vault is bound to the wrong Lite; do NOT wire, redeploy");
}
const liteAddr = predictedLite, feeVaultAddr = predictedFeeVault;

async function ensure(label: string, isDone: () => Promise<boolean>, act: () => Promise<ethers.ContractTransactionResponse>) {
  if (await isDone()) { console.log(`  ✓ ${label} (already)`); return; }
  await (await act()).wait();
  if (!(await isDone())) throw new Error(`${label} landed but the getter still disagrees`);
  console.log(`  ✓ ${label}`);
}
await ensure(`Outbox.setTrustedForwarder(${liteAddr}, true)`,
  () => outbox.isTrustedForwarder(liteAddr), () => outbox.setTrustedForwarder(liteAddr, true));
await ensure(`Lite.setRelayerFeeVault(${feeVaultAddr})`,
  async () => same(await lite.relayerFeeVault(), feeVaultAddr), () => lite.setRelayerFeeVault(feeVaultAddr));
await ensure(`Lite.setDestinationEvmChainId(${CHAIN_KEY}, ${DEST_CHAIN_ID})`,
  async () => Number(await lite.destinationEvmChainIds(CHAIN_KEY)) === DEST_CHAIN_ID, () => lite.setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID));
await ensure(`Lite.setAuthorizedQuoter(${QUOTER_EOA}, true)`,
  () => lite.authorizedQuoters(QUOTER_EOA), () => lite.setAuthorizedQuoter(QUOTER_EOA, true));
await ensure(`Decoder.setTrustedInbox(${DEST_CHAIN_ID}, ${dst.inbox})`,
  async () => same(await decoder.trustedInboxes(DEST_CHAIN_ID), dst.inbox), () => decoder.setTrustedInbox(DEST_CHAIN_ID, dst.inbox));

if (src.relayerContract && !same(src.relayerContract, liteAddr)) src.oldRelayerContract = src.relayerContract;
if (src.relayerFeeVault && !same(src.relayerFeeVault, feeVaultAddr)) src.oldRelayerFeeVault = src.relayerFeeVault;
src.relayerContract = liteAddr;
src.relayerContractKind = "RelayerContractLite";
src.relayerFeeVault = feeVaultAddr;
src.quoterEOA = QUOTER_EOA;
src.liteRedeployedAt = new Date().toISOString();
writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
console.log(`✅ chain key ${CHAIN_KEY}: RelayerContractLite ${liteAddr}  RelayerFeeVault ${feeVaultAddr}  (owner ${owner}). Written to ${OUT}.`);
console.log("Next: relayer IaC route relayerContractAddress → new Lite, rollout; emitters approveForwarder(newLite).");
provider.destroy();
