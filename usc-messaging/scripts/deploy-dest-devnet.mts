// usc-dev destination-stack deploy (real Sepolia, chain key 8): MockDestination + DefaultDispatcher
// + DispatcherRouter + Inbox, REUSING the live EOAValidator (attestors + relayer already speak it).
//
// asc-contracts #36 (main a9791c37): the Inbox's messageDispatcher is the DispatcherRouter, not the
// dApp; publishers wrap their memo in the EVM envelope abi.encode(dest.dapp, 0, gasLimit, memo)
// (see evm-envelope.mts). Deploy order / constructor args: dispatcher-stack.mts.
//
// The Inbox allowlists source.outbox at construction when the deploy JSON already has one (#48);
// otherwise deploy-source-devnet.mts must call Inbox.setSupportedOutbox afterwards. Follow up with
// repoint-inbox-devnet.mts so the Creditcoin-side ack validator + delivery decoder trust the new Inbox.
//
// Env: SEPOLIA_RPC, DEPLOYER_KEY (becomes Inbox/router owner), DEPLOY_OUT. Artifacts from
// $ASC_CONTRACTS_DIR (main a9791c37+ build). DEVNET-ONLY keys.
import { ethers } from "ethers";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { deployRouterStackWithInbox, ensureDestinationTrusts } from "./dispatcher-stack.mjs";

function ascContractsDir(): string {
  // ASC_CONTRACTS_DIR since the repo was renamed usc-contracts -> asc-contracts; the old name is
  // still accepted so existing setups keep working. Deliberately no default: this used to fall
  // back to one developer's home directory, so anyone else got a confusing "cannot read artifact"
  // three calls later instead of being told what to set.
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) {
    throw new Error(
      "set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (run `npx hardhat compile` there first)",
    );
  }
  return dir;
}

const UC = ascContractsDir();
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = process.env.DEPLOY_OUT ?? "./usc-dev-deploy.json";
const CHAIN_KEY = 8;
const CREDITCOIN_CHAIN_ID = 42;
const SEPOLIA_CHAIN_ID = 11155111;
const VALIDATOR = "0x71A21Ea8d28D3a0618d61d478Ee20DCB64be8082"; // live EOAValidator — reused
const LOCAL_CHAIN_KEY = ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32);

const rpc = process.env.SEPOLIA_RPC!;
const key = process.env.DEPLOYER_KEY!;
if (!rpc || !key) throw new Error("need SEPOLIA_RPC + DEPLOYER_KEY");

const provider = new ethers.JsonRpcProvider(rpc, SEPOLIA_CHAIN_ID, { staticNetwork: true });
const wallet = new ethers.Wallet(key, provider);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

const addrs = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
// Allowlist at construction (Inbox #48 six-arg ctor): INITIAL_OUTBOXES (comma-separated) when set,
// else the already-deployed Creditcoin Outbox from the deploy JSON, else empty (owner calls
// setSupportedOutbox(outbox, true) after the source deploy).
const initialOutboxes: string[] = process.env.INITIAL_OUTBOXES
  ? process.env.INITIAL_OUTBOXES.split(",").map((a) => a.trim()).filter(Boolean).map((a) => ethers.getAddress(a))
  : addrs.source?.outbox ? [ethers.getAddress(addrs.source.outbox)] : [];

console.log("deployer:", wallet.address, "balance:", ethers.formatEther(await provider.getBalance(wallet.address)), "ETH");
console.log("initialOutboxes:", initialOutboxes.length ? initialOutboxes.join(", ") : "(none — run Inbox.setSupportedOutbox after the source deploy)");
const dapp = await deploy("MockDestination", ART("mocks/TestMocks.sol", "MockDestination"));
const stack = await deployRouterStackWithInbox({
  wallet, ART,
  inboxArgsFor: (router) => [LOCAL_CHAIN_KEY, CREDITCOIN_CHAIN_ID, VALIDATOR, router, wallet.address, initialOutboxes],
});
await ensureDestinationTrusts(wallet, await dapp.getAddress(), {
  DispatcherRouter: stack.dispatcherRouter, DefaultDispatcher: stack.defaultDispatcher,
});

addrs.dest = {
  chainId: SEPOLIA_CHAIN_ID, chainKey: CHAIN_KEY, creditcoinChainId: CREDITCOIN_CHAIN_ID,
  localChainKey: LOCAL_CHAIN_KEY, voteValidator: VALIDATOR,
  inbox: stack.inbox, dispatcherRouter: stack.dispatcherRouter, defaultDispatcher: stack.defaultDispatcher,
  dapp: await dapp.getAddress(), admin: wallet.address, deployedAt: new Date().toISOString(),
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log("✅ dest stack deployed →", OUT);
console.log("   Next: repoint-inbox-devnet.mts NEW_INBOX=" + stack.inbox + " (ack validator + delivery decoder), then roll the relayer IaC inboxAddress.");
process.exit(0);
