// DefaultDispatcher + DispatcherRouter deploy/wiring shared by the destination-stack scripts
// (asc-contracts #36, main a9791c37). Mirrors scripts/hardhat/deployWriteAbility.ts §"Dispatcher
// router stack" (lines ~1113-1194) minus the CREATE2 deployer and the RateLimitDispatcher, which
// the e2e / devnet stacks do not need.
//
// Constructor circularity: DefaultDispatcher(inbox, owner) and DispatcherRouter(inbox, owner,
// defaultDispatcher, initialDispatchers) both pin the Inbox as their trusted caller, while
// Inbox(chainKey, sourceChainId, validator, messageDispatcher, owner, initialOutboxes) requires
// `messageDispatcher` (the router) to already have code. The hardhat script resolves this with a
// CREATE2 Inbox; here the deployer's nonces are consecutive so plain CREATE prediction works:
//
//   n     DefaultDispatcher(predictedInbox, owner)
//   n+1   DispatcherRouter(predictedInbox, owner, defaultDispatcher, [])   // registers + sets default in ctor
//   n+2   Inbox(..., router, ...)
//   then  DefaultDispatcher.setRouter(router)                                // deployWriteAbility.ts:1177
//         destination.setTrustedInbox(router, true)                           // ensureDestinationTrustsCaller, :1186
//
// The router's constructor already does `_registerDispatcher(default)` + `defaultDispatcher =
// default` (DispatcherRouter.sol:50-52), so the explicit registerDispatcher/setDefaultDispatcher
// calls are only issued when a read-back shows them missing (keeps re-runs idempotent).
import { ethers } from "ethers";

export type Artifact = { abi: ethers.InterfaceAbi; bytecode: string };
export type ArtifactLoader = (path: string, name: string) => Artifact;

export interface RouterStack {
  defaultDispatcher: string;
  dispatcherRouter: string;
  inbox: string;
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

async function deployContract(wallet: ethers.Wallet, name: string, art: Artifact, args: unknown[]): Promise<ethers.Contract> {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c as unknown as ethers.Contract;
}

/// Deploy DefaultDispatcher + DispatcherRouter + a NEW Inbox whose messageDispatcher is the router.
/// `inboxArgsFor(router)` returns the six Inbox constructor args with the router in slot 4.
export async function deployRouterStackWithInbox(opts: {
  wallet: ethers.Wallet;
  ART: ArtifactLoader;
  inboxArgsFor: (router: string) => unknown[];
}): Promise<RouterStack> {
  const { wallet, ART } = opts;
  const owner = wallet.address;
  const n = await wallet.getNonce("pending");
  const predictedRouter = ethers.getCreateAddress({ from: owner, nonce: n + 1 });
  const predictedInbox = ethers.getCreateAddress({ from: owner, nonce: n + 2 });

  const dflt = await deployContract(wallet, "DefaultDispatcher", ART("write-ability/DefaultDispatcher.sol", "DefaultDispatcher"),
    [predictedInbox, owner]);
  const router = await deployContract(wallet, "DispatcherRouter", ART("write-ability/DispatcherRouter.sol", "DispatcherRouter"),
    [predictedInbox, owner, await dflt.getAddress(), []]);
  if (!same(await router.getAddress(), predictedRouter)) {
    throw new Error(`DispatcherRouter landed at ${await router.getAddress()}, predicted ${predictedRouter} (nonce moved under us — do not share the deployer key while this runs)`);
  }
  const inboxArgs = opts.inboxArgsFor(predictedRouter);
  if (!same(String(inboxArgs[3]), predictedRouter)) throw new Error("inboxArgsFor must place the router in Inbox ctor slot 4 (messageDispatcher)");
  const inbox = await deployContract(wallet, "Inbox", ART("write-ability/Inbox.sol", "Inbox"), inboxArgs);
  if (!same(await inbox.getAddress(), predictedInbox)) {
    throw new Error(`Inbox landed at ${await inbox.getAddress()}, predicted ${predictedInbox}; the dispatchers trust the wrong address — redeploy`);
  }

  const stack = { defaultDispatcher: await dflt.getAddress(), dispatcherRouter: predictedRouter, inbox: predictedInbox };
  await wireRouter(wallet, ART, stack);
  return stack;
}

/// Deploy DefaultDispatcher + DispatcherRouter for an EXISTING Inbox (no address prediction needed:
/// both take the real Inbox address). Does NOT call Inbox.setMessageDispatcher — the caller owns that.
export async function deployRouterStackForInbox(opts: {
  wallet: ethers.Wallet;
  ART: ArtifactLoader;
  inbox: string;
}): Promise<RouterStack> {
  const { wallet, ART, inbox } = opts;
  const owner = wallet.address;
  const dflt = await deployContract(wallet, "DefaultDispatcher", ART("write-ability/DefaultDispatcher.sol", "DefaultDispatcher"), [inbox, owner]);
  const router = await deployContract(wallet, "DispatcherRouter", ART("write-ability/DispatcherRouter.sol", "DispatcherRouter"),
    [inbox, owner, await dflt.getAddress(), []]);
  const stack = { defaultDispatcher: await dflt.getAddress(), dispatcherRouter: await router.getAddress(), inbox };
  await wireRouter(wallet, ART, stack);
  return stack;
}

/// Post-construction wiring + read-back, idempotent: DefaultDispatcher.setRouter(router), and the
/// router's registration/default of the DefaultDispatcher (normally already done by its ctor).
export async function wireRouter(wallet: ethers.Wallet, ART: ArtifactLoader, s: RouterStack): Promise<void> {
  const dflt = new ethers.Contract(s.defaultDispatcher, ART("write-ability/DefaultDispatcher.sol", "DefaultDispatcher").abi, wallet);
  const router = new ethers.Contract(s.dispatcherRouter, ART("write-ability/DispatcherRouter.sol", "DispatcherRouter").abi, wallet);

  if (!same(await dflt.router(), s.dispatcherRouter)) {
    await (await dflt.setRouter(s.dispatcherRouter)).wait();
    console.log("  DefaultDispatcher.setRouter(DispatcherRouter)");
  } else console.log("  DefaultDispatcher.router() already = DispatcherRouter");

  if (!(await router.registeredDispatchers(s.defaultDispatcher))) {
    await (await router.registerDispatcher(s.defaultDispatcher)).wait();
    console.log("  DispatcherRouter.registerDispatcher(DefaultDispatcher)");
  }
  if (!same(await router.defaultDispatcher(), s.defaultDispatcher)) {
    await (await router.setDefaultDispatcher(s.defaultDispatcher)).wait();
    console.log("  DispatcherRouter.setDefaultDispatcher(DefaultDispatcher)");
  } else console.log("  DispatcherRouter.defaultDispatcher() already = DefaultDispatcher (set in ctor)");

  if (!(await router.isTrustedInbox(s.inbox))) throw new Error(`DispatcherRouter does not trust Inbox ${s.inbox}`);
  if (!(await dflt.isTrustedInbox(s.inbox))) throw new Error(`DefaultDispatcher does not trust Inbox ${s.inbox}`);
}

/// Destination calls see the router (or, for direct Inbox→DefaultDispatcher wiring, the
/// DefaultDispatcher) as msg.sender. MockDestination's fallback does not check trust, but the
/// hardhat reference bootstraps it anyway (deployWriteAbility.ts ensureDestinationTrustsCaller;
/// test/DispatcherRouter.ts:513-514 trusts both), so do the same — fail closed if the destination
/// lacks the ITrustedInbox surface, as the hardhat script does.
export async function ensureDestinationTrusts(wallet: ethers.Wallet, destination: string, callers: Record<string, string>): Promise<void> {
  const dest = new ethers.Contract(destination, [
    "function isTrustedInbox(address) view returns (bool)",
    "function setTrustedInbox(address inbox, bool trusted)",
  ], wallet);
  for (const [label, caller] of Object.entries(callers)) {
    let trusted: boolean;
    try {
      trusted = Boolean(await dest.isTrustedInbox(caller));
    } catch {
      throw new Error(`destination ${destination} does not implement isTrustedInbox — its owner must make it trust ${label} ${caller}`);
    }
    if (trusted) { console.log(`  destination already trusts ${label}`); continue; }
    await (await dest.setTrustedInbox(caller, true)).wait();
    if (!(await dest.isTrustedInbox(caller))) throw new Error(`destination.setTrustedInbox(${label}, true) did not stick`);
    console.log(`  destination.setTrustedInbox(${label}, true)`);
  }
}
