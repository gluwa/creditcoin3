// usc-dev: put the asc-contracts #36 DispatcherRouter in front of an EXISTING Sepolia Inbox.
//
// The live chain-key-8 Inbox (dest.inbox in usc-dev-deploy.json) was deployed with MockDestination
// as its messageDispatcher. Since #36 (main a9791c37) the Inbox calls IMessageDispatcher.deliverMessage
// (bytes32 messageId, address emitter, bytes messagePayload) on that address and expects the payload
// to be the EVM envelope abi.encode(destination, nativeCoinValue, gasLimit, payloadData); the
// DispatcherRouter is the contract that decodes it and calls `destination`. Rather than redeploy
// the Inbox (and repoint the ack validator / delivery decoder / relayer IaC), swap its dispatcher:
//
//   DefaultDispatcher(inbox, owner)                              // DefaultDispatcher.sol:11
//   DispatcherRouter(inbox, owner, defaultDispatcher, [])        // DispatcherRouter.sol:42 — registers + sets default in ctor
//   DefaultDispatcher.setRouter(router)                          // deployWriteAbility.ts:1177
//   [DispatcherRouter.registerDispatcher / setDefaultDispatcher only if a read-back shows them missing]
//   Inbox.setMessageDispatcher(router)                           // Inbox.sol:238, onlyOwner
//   MockDestination.setTrustedInbox(router, true) (+ DefaultDispatcher)  // ensureDestinationTrustsCaller
//
// Idempotent: re-runs reuse dest.dispatcherRouter / dest.defaultDispatcher when recorded and still
// deployed, and every wiring step is check-then-set. Afterwards publishers must send the envelope
// (publish-lite-devnet.mts / publish-devnet.mts already do, with dest.dapp as destination).
//
// Env:
//   SEPOLIA_RPC        destination RPC                                        (required)
//   INBOX_OWNER_KEY    owner of dest.inbox; becomes owner of the new router + dispatcher (required)
//   ASC_CONTRACTS_DIR  compiled asc-contracts checkout (main a9791c37+)         (required)
//   DEPLOY_OUT         default ../usc-dev-deploy.json
//
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { deployRouterStackForInbox, ensureDestinationTrusts, wireRouter, type RouterStack } from "./dispatcher-stack.mjs";

function ascContractsDir(): string {
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main a9791c37+, `npx hardhat compile`)");
  return dir;
}
const UC = ascContractsDir();
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
for (const k of ["SEPOLIA_RPC", "INBOX_OWNER_KEY"]) if (!process.env[k]) throw new Error(`missing ${k}`);

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
const dest = deployJson.dest;
if (!dest?.inbox) throw new Error(`no dest.inbox in ${OUT}`);
if (!dest?.dapp) throw new Error(`no dest.dapp in ${OUT} — the envelope destination the router must be trusted by`);
const INBOX = ethers.getAddress(dest.inbox);
const DAPP = ethers.getAddress(dest.dapp);
const DEST_CHAIN_ID = Number(dest.chainId ?? 11155111);

const provider = new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC!, DEST_CHAIN_ID, { staticNetwork: true });
const wallet = new ethers.Wallet(process.env.INBOX_OWNER_KEY!, provider);
// staticNetwork skips the handshake, so confirm we are really on the recorded chain before spending gas.
const liveChainId = Number(BigInt(await provider.send("eth_chainId", [])));
if (liveChainId !== DEST_CHAIN_ID) throw new Error(`RPC reports chain id ${liveChainId}, dest.chainId is ${DEST_CHAIN_ID}`);

const inbox = new ethers.Contract(INBOX, ART("write-ability/Inbox.sol", "Inbox").abi, wallet);
const inboxOwner: string = await inbox.owner();
if (!same(inboxOwner, wallet.address)) throw new Error(`Inbox ${INBOX} owner is ${inboxOwner}, but INBOX_OWNER_KEY is ${wallet.address}`);
const before: string = await inbox.messageDispatcher();
console.log(`Inbox ${INBOX} (chain ${DEST_CHAIN_ID}) owner ${wallet.address}; messageDispatcher now ${before}${same(before, DAPP) ? " (= dest.dapp, pre-#36 wiring)" : ""}`);
console.log(`deployer balance ${ethers.formatEther(await provider.getBalance(wallet.address))} ETH`);

// Reuse a recorded router if it is still deployed and actually belongs to this Inbox.
let stack: RouterStack | undefined;
if (dest.dispatcherRouter && dest.defaultDispatcher) {
  const [routerCode, dfltCode] = await Promise.all([provider.getCode(dest.dispatcherRouter), provider.getCode(dest.defaultDispatcher)]);
  if (routerCode !== "0x" && dfltCode !== "0x") {
    const router = new ethers.Contract(dest.dispatcherRouter, ART("write-ability/DispatcherRouter.sol", "DispatcherRouter").abi, provider);
    if (!(await router.isTrustedInbox(INBOX))) throw new Error(`recorded dest.dispatcherRouter ${dest.dispatcherRouter} does not trust Inbox ${INBOX}; remove it from ${OUT} to redeploy`);
    stack = { dispatcherRouter: ethers.getAddress(dest.dispatcherRouter), defaultDispatcher: ethers.getAddress(dest.defaultDispatcher), inbox: INBOX };
    console.log(`reusing recorded DispatcherRouter ${stack.dispatcherRouter} / DefaultDispatcher ${stack.defaultDispatcher}`);
    await wireRouter(wallet, ART, stack);
  } else console.log("recorded dispatcherRouter/defaultDispatcher have no code on this chain — deploying fresh");
}
if (!stack) stack = await deployRouterStackForInbox({ wallet, ART, inbox: INBOX });

// Destination trust: the router (and, for direct wiring, the DefaultDispatcher) is msg.sender at the dApp.
await ensureDestinationTrusts(wallet, DAPP, { DispatcherRouter: stack.dispatcherRouter, DefaultDispatcher: stack.defaultDispatcher });

// Swap the Inbox's dispatcher last, once the router is fully wired.
if (!same(await inbox.messageDispatcher(), stack.dispatcherRouter)) {
  await (await inbox.setMessageDispatcher(stack.dispatcherRouter)).wait();
  console.log(`  Inbox.setMessageDispatcher(${stack.dispatcherRouter})`);
} else console.log("  Inbox.messageDispatcher() already = DispatcherRouter");
const after: string = await inbox.messageDispatcher();
if (!same(after, stack.dispatcherRouter)) throw new Error(`Inbox.messageDispatcher() is ${after}, expected ${stack.dispatcherRouter}`);

deployJson.dest = {
  ...dest,
  dispatcherRouter: stack.dispatcherRouter,
  defaultDispatcher: stack.defaultDispatcher,
  previousMessageDispatcher: same(before, stack.dispatcherRouter) ? dest.previousMessageDispatcher : before,
  routerDeployedAt: dest.routerDeployedAt && same(dest.dispatcherRouter ?? "", stack.dispatcherRouter) ? dest.routerDeployedAt : new Date().toISOString(),
};
writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
console.log(`✅ Inbox ${INBOX} now dispatches through DispatcherRouter ${stack.dispatcherRouter} (default impl ${stack.defaultDispatcher}) → recorded in ${OUT}`);
console.log("   Publishers must now send the #36 envelope abi.encode(dest.dapp, 0, gasLimit, memo) — publish-lite-devnet.mts does.");
process.exit(0);
