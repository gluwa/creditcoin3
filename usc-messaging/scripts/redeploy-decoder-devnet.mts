// devnet: redeploy EVMDeliveryDecoder and point every RelayerContractLite at it.
//
// Why: the decoder bakes in the selector of Inbox.deliverMessage, so every Inbox ABI change
// (asc-contracts #48: 5-arg 0x1d36ca7b; #54: sequence arg, 0x230ceea8) makes `claimDelivery` on
// every route revert `UnsupportedDestinationTransaction()` (0x2943653a) even though delivery + ack
// work. The current selector is read from the compiled Inbox ABI in ASC_CONTRACTS_DIR and compared
// with source.deliveryDecoderSelector in the deploy record: deploy a new decoder when they differ,
// trust each route's current Inbox, and swap it into the deployer-owned Lites.
//
// Routes come from the record: usc-devnet's legacy root route (source/dest) when present, plus every
// chains.<K> entry with dest.inbox + source.relayerContract (all of asc-devnet's routes).
//
// Idempotent: re-running with the recorded decoder already on the current selector only fixes
// missing trust entries / Lite pointers. Set NEW_DECODER=0x… to adopt an already deployed decoder.
//
// Env: NETWORK (asc-devnet default | usc-devnet), DEPLOYER_KEY (owner of the Lites and the decoder),
//      ASC_CONTRACTS_DIR (compiled asc-contracts main), optional CC_RPC, DEPLOY_OUT, NEW_DECODER.
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { network, deployPath, ccRpc, ccProvider } from "./network.mjs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
if (!UC) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+, `npx hardhat compile`)");
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const NET = network();
const OUT = deployPath(NET);
const CC_CHAIN_ID = NET.evmChainId;
if (!process.env.DEPLOYER_KEY) throw new Error("missing DEPLOYER_KEY");

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const s = addrs.source;
const { provider } = await ccProvider(NET, s.rpc);
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY, provider);
console.log("deployer:", wallet.address, "balance:", ethers.formatEther(await provider.getBalance(wallet.address)), "CTC");

const decoderArt = ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder");
// The decoder bakes in Inbox.deliverMessage's selector, so "current" means the selector of the
// compiled Inbox ABI in ASC_CONTRACTS_DIR, not a constant (#48: 0x1d36ca7b, #54: 0x230ceea8, …).
const expectedSelector: string = new ethers.Interface(ART("write-ability/Inbox.sol", "Inbox").abi).getFunction("deliverMessage")!.selector;
console.log("deliverMessage selector in the compiled Inbox ABI:", expectedSelector, "recorded:", s.deliveryDecoderSelector ?? "none");
const liteAbi = [
  "function owner() view returns (address)",
  "function deliveryDecoder() view returns (address)",
  "function setDeliveryDecoder(address newDeliveryDecoder)",
];

// Routes: chain key -> { destChainId, inbox, lite }
type Route = { chainKey: number; destChainId: number; inbox: string; lite: string };
// usc-devnet's record keeps its first route (Sepolia, key 8) at the root (source/dest); every other
// route, and all of asc-devnet's, live under chains.<K>.
const legacyRoute: Route[] = addrs.dest?.inbox && s.relayerContract
  ? [{ chainKey: Number(s.chainKey ?? 8), destChainId: Number(addrs.dest.chainId ?? 11155111), inbox: addrs.dest.inbox, lite: s.relayerContract }]
  : [];
const routes: Route[] = [
  ...legacyRoute,
  ...Object.values(addrs.chains ?? {}).map((c: any) => ({
    chainKey: Number(c.chainKey), destChainId: Number(c.dest?.chainId ?? c.destChainId), inbox: c.dest?.inbox, lite: c.source?.relayerContract,
  })),
].filter((r) => r.inbox && r.lite && Number.isInteger(r.destChainId) && r.destChainId > 0);
if (routes.length === 0) throw new Error(`no routes with dest.inbox + source.relayerContract in ${OUT} — nothing to trust / repoint`);
for (const r of routes) console.log(`  route ${r.chainKey}: destChainId ${r.destChainId} inbox ${r.inbox} lite ${r.lite}`);

// 1. decoder
let decoderAddr: string;
if (process.env.NEW_DECODER) {
  decoderAddr = ethers.getAddress(process.env.NEW_DECODER);
  console.log("adopting decoder", decoderAddr);
} else if (s.deliveryDecoder && s.deliveryDecoderSelector === expectedSelector) {
  decoderAddr = s.deliveryDecoder;
  console.log(`decoder already targets ${expectedSelector}:`, decoderAddr);
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
if (s.deliveryDecoder && s.deliveryDecoder.toLowerCase() !== decoderAddr.toLowerCase()) s.oldDeliveryDecoder = s.deliveryDecoder;
s.deliveryDecoder = decoderAddr;
s.deliveryDecoderSelector = expectedSelector;
s.deliveryDecoderRedeployedAt = new Date().toISOString();
writeFileSync(OUT, JSON.stringify(addrs, null, 2) + "\n");
console.log("✅ decoder", decoderAddr, "trusted on", routes.length, "routes; written to", OUT);
provider.destroy();
