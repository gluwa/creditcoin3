// Registers one spoke chain (Base Sepolia or Ethereum Sepolia) against an already-deployed
// BridgeHub: deploys BridgeVault on the spoke chain, trusting that chain's existing, shared
// DispatcherRouter (not a dedicated Inbox of our own), and wires BridgeHub.setChainConfig to
// publish through that chain's existing, shared Outbox.
//
// Run once per spoke chain key. For a single-chain dry run, run this once with chainKey=8
// (Ethereum Sepolia) — that single vault then serves as both source and destination of its own
// deposits.
//
// Why no dedicated Inbox/Outbox of our own: DestinationCall (asc-contracts write-ability/common/
// DestinationCall.sol), the router's execution primitive, makes a plain external call to whatever
// destination address a message's payload encodes, with the attested emitter appended as trailing
// calldata — there's no allowlist on the destination, so any already-attested message can target
// our own BridgeVault directly. Outbox.publishMessage is likewise permissionless (any caller). The
// live devnet's Inbox/Outbox/message-relayer for chain keys 8 and 9 already exist and work; this
// reuses them entirely rather than standing up parallel dedicated infra. See BridgeVault.sol's and
// BridgeHub.sol's NatSpec for the full mechanism.
//
// Usage: tsx deploy-bridge-spoke.mts <chainKey>
// Env: SPOKE_RPC, DEPLOYER_KEY (controls both the spoke and Creditcoin wallets), SPOKE_CHAIN_ID,
//      SHARED_ROUTER (that spoke's live DispatcherRouter address — this chain's Inbox.
//      messageDispatcher()), SHARED_OUTBOX (that spoke's live, canonical Outbox address on
//      Creditcoin, i.e. the one already allowlisted on that chain's Inbox and already watched by
//      the running message-relayer), CC_RPC (default devnet), CREDITCOIN_CHAIN_ID (default 42),
//      DEPLOY_OUT (default ./usc-bridge-deploy.json, must already have .hub from deploy-bridge-hub).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const ART_FOUNDRY = (contract: string) => {
  const j = JSON.parse(
    readFileSync(
      `${import.meta.dirname}/../contracts/out/${contract}.sol/${contract}.json`,
      "utf8",
    ),
  );
  return { abi: j.abi, bytecode: j.bytecode.object as string };
};

const OUT = process.env.DEPLOY_OUT ?? "./usc-bridge-deploy.json";
const CC_CHAIN_ID = Number(process.env.CREDITCOIN_CHAIN_ID ?? 42);

const chainKeyArg = process.argv[2];
if (!chainKeyArg) throw new Error("usage: deploy-bridge-spoke.mts <chainKey>");
const CHAIN_KEY = Number(chainKeyArg);
if (!Number.isInteger(CHAIN_KEY) || CHAIN_KEY <= 0 || CHAIN_KEY > 0xffff) {
  throw new Error(`chainKey must be a positive uint16, got ${chainKeyArg}`);
}

const spokeRpc = process.env.SPOKE_RPC!;
const key = process.env.DEPLOYER_KEY!;
const spokeChainId = Number(process.env.SPOKE_CHAIN_ID ?? NaN);
const sharedRouter = process.env.SHARED_ROUTER!;
const sharedOutbox = process.env.SHARED_OUTBOX!;
const ccRpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
if (
  !spokeRpc ||
  !key ||
  !Number.isInteger(spokeChainId) ||
  !sharedRouter ||
  !sharedOutbox
) {
  throw new Error(
    "need SPOKE_RPC, DEPLOYER_KEY, SPOKE_CHAIN_ID, SHARED_ROUTER, SHARED_OUTBOX",
  );
}

const spokeProvider = new ethers.JsonRpcProvider(spokeRpc, spokeChainId, {
  staticNetwork: true,
});
const spokeWallet = new ethers.Wallet(key, spokeProvider);
const ccProvider = new ethers.JsonRpcProvider(ccRpc, CC_CHAIN_ID, {
  staticNetwork: true,
  polling: true,
});
const ccWallet = new ethers.Wallet(key, ccProvider);

async function main() {
  const out = JSON.parse(readFileSync(OUT, "utf8"));
  const hubAddr = out.hub?.bridgeHub as string | undefined;
  if (!hubAddr) {
    throw new Error(
      `${OUT}: hub.bridgeHub missing — run deploy-bridge-hub.mts first`,
    );
  }

  console.log(
    "spoke deployer:",
    spokeWallet.address,
    "balance:",
    ethers.formatEther(await spokeProvider.getBalance(spokeWallet.address)),
  );

  const vaultFactory = new ethers.ContractFactory(
    ART_FOUNDRY("BridgeVault").abi,
    ART_FOUNDRY("BridgeVault").bytecode,
    spokeWallet,
  );
  const vault = await vaultFactory.deploy(
    sharedRouter,
    spokeWallet.address,
    hubAddr,
  );
  const vaultAddr = await vault.getAddress();
  console.log(
    `  BridgeVault → ${vaultAddr} (trusts shared router ${sharedRouter})`,
  );
  await vault.waitForDeployment();

  const hub = new ethers.Contract(
    hubAddr,
    ART_FOUNDRY("BridgeHub").abi,
    ccWallet,
  );
  await (
    await (hub as any).setChainConfig(CHAIN_KEY, vaultAddr, sharedOutbox, true)
  ).wait();
  console.log(
    `  BridgeHub.setChainConfig(${CHAIN_KEY}, vault=${vaultAddr}, outbox=${sharedOutbox}, enabled=true)`,
  );

  out.spokes = out.spokes ?? {};
  out.spokes[CHAIN_KEY] = {
    chainId: spokeChainId,
    rpc: spokeRpc,
    chainKey: CHAIN_KEY,
    admin: spokeWallet.address,
    bridgeVault: vaultAddr,
    sharedRouter,
    sharedOutbox,
  };
  writeFileSync(OUT, JSON.stringify(out, null, 2));
  console.log(`✅ spoke chainKey=${CHAIN_KEY} registered → ${OUT}`);
  console.log(
    "   nothing else to configure — the existing message-relayer already delivers to",
    sharedRouter,
    "for this chain key",
  );
  process.exit(0);
}
main().catch((e) => {
  console.error("FAILED:", e.shortMessage ?? e.message);
  process.exit(1);
});
