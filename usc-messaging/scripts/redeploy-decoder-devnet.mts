// Redeploy EVMDeliveryDecoder on usc-devnet and point every RelayerContractLite at it.
//
// Why: the decoder deployed on 2026-08-06 (source.deliveryDecoder) hard-codes the selector of the
// pre-#48 `deliverMessage(bytes32,address,bytes,bytes)`. Since asc-contracts #48 the Inbox exposes
// `deliverMessage(bytes32,address,address,bytes,bytes)` (0x1d36ca7b), so `claimDelivery` on every
// route reverts `UnsupportedDestinationTransaction()` (0x2943653a) even though delivery + ack work.
// The decoder source on asc-contracts main (c83b3372 and a9791c37 are identical for this file) already
// carries the new selector: deploy it, trust each route's current Inbox, and swap it into the Lites.
//
// Idempotent: re-running with source.deliveryDecoder already on the new selector only fixes missing
// trust entries / Lite pointers. Set NEW_DECODER=0x… to adopt an already deployed decoder instead of
// deploying another one.
//
// Env: DEPLOYER_KEY (owner of the Lites and the decoder), ASC_CONTRACTS_DIR (compiled main c83b3372+),
//      optional CC_RPC, DEPLOY_OUT, NEW_DECODER.
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
if (!UC) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+, `npx hardhat compile`)");
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const CC_CHAIN_ID = 42;
if (!process.env.DEPLOYER_KEY) throw new Error("missing DEPLOYER_KEY");

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const s = addrs.source;
const provider = new ethers.JsonRpcProvider(process.env.CC_RPC ?? s.rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY, provider);
console.log("deployer:", wallet.address, "balance:", ethers.formatEther(await provider.getBalance(wallet.address)), "CTC");

const decoderArt = ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder");
const liteAbi = [
  "function owner() view returns (address)",
  "function deliveryDecoder() view returns (address)",
  "function setDeliveryDecoder(address newDeliveryDecoder)",
];

// Routes: chain key -> { destChainId, inbox, lite }
type Route = { chainKey: number; destChainId: number; inbox: string; lite: string };
const routes: Route[] = [
  { chainKey: 8, destChainId: 11155111, inbox: addrs.dest.inbox, lite: s.relayerContract },
  ...Object.values(addrs.chains ?? {}).map((c: any) => ({
    chainKey: c.chainKey, destChainId: c.destChainId, inbox: c.dest?.inbox, lite: c.source?.relayerContract,
  })),
].filter((r) => r.inbox && r.lite);
for (const r of routes) console.log(`  route ${r.chainKey}: destChainId ${r.destChainId} inbox ${r.inbox} lite ${r.lite}`);

// 1. decoder
let decoderAddr: string;
if (process.env.NEW_DECODER) {
  decoderAddr = ethers.getAddress(process.env.NEW_DECODER);
  console.log("adopting decoder", decoderAddr);
} else if (s.deliveryDecoderSelector === "0x1d36ca7b") {
  decoderAddr = s.deliveryDecoder;
  console.log("decoder already on the 5-arg selector:", decoderAddr);
} else {
  const f = new ethers.ContractFactory(decoderArt.abi, decoderArt.bytecode, wallet);
  const c = await f.deploy(wallet.address);
  decoderAddr = await c.getAddress();
  console.log("EVMDeliveryDecoder →", decoderAddr);
  await c.waitForDeployment();
}
const decoder = new ethers.Contract(decoderAddr, decoderArt.abi, wallet);
if ((await decoder.owner()).toLowerCase() !== wallet.address.toLowerCase()) throw new Error("decoder owner is not the deployer");

// 2. trust each route's current Inbox
for (const r of routes) {
  const cur: string = await decoder.trustedInboxes(r.destChainId);
  if (cur.toLowerCase() === r.inbox.toLowerCase()) {
    console.log(`  decoder already trusts ${r.inbox} for chain id ${r.destChainId}`);
    continue;
  }
  console.log(`  decoder.setTrustedInbox(${r.destChainId}, ${r.inbox}) (was ${cur})`);
  await (await decoder.setTrustedInbox(r.destChainId, r.inbox)).wait();
}

// 3. swap the decoder into every Lite
for (const r of routes) {
  const lite = new ethers.Contract(r.lite, liteAbi, wallet);
  if ((await lite.owner()).toLowerCase() !== wallet.address.toLowerCase()) {
    console.log(`  ! Lite ${r.lite} (route ${r.chainKey}) is not owned by the deployer — skipping`);
    continue;
  }
  const cur: string = await lite.deliveryDecoder();
  if (cur.toLowerCase() === decoderAddr.toLowerCase()) {
    console.log(`  Lite ${r.lite} already uses the new decoder`);
    continue;
  }
  console.log(`  Lite ${r.lite}.setDeliveryDecoder(${decoderAddr}) (was ${cur})`);
  await (await lite.setDeliveryDecoder(decoderAddr)).wait();
  if ((await lite.deliveryDecoder()).toLowerCase() !== decoderAddr.toLowerCase()) throw new Error("setDeliveryDecoder did not land");
}

// 4. record
if (s.deliveryDecoder.toLowerCase() !== decoderAddr.toLowerCase()) s.oldDeliveryDecoder = s.deliveryDecoder;
s.deliveryDecoder = decoderAddr;
s.deliveryDecoderSelector = "0x1d36ca7b"; // deliverMessage(bytes32,address,address,bytes,bytes)
s.deliveryDecoderRedeployedAt = new Date().toISOString();
writeFileSync(OUT, JSON.stringify(addrs, null, 2) + "\n");
console.log("✅ decoder", decoderAddr, "trusted on", routes.length, "routes; written to", OUT);
provider.destroy();
