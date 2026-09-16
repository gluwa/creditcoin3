// Standalone daemon: watches every configured spoke route for `Deposited` events and submits the
// matching `BridgeHub.claim` on Creditcoin. `BridgeHub.claim` is fully permissionless and
// `claimed`-gated, so this bot is a convenience, not a trust boundary — anyone (a user, a script)
// may also claim directly, and this bot racing them is a safe no-op.
//
// ⚠️ POC scope, matching dApp-ack-worker's own disclaimer: no durable checkpoint store. On restart
// each route rewinds a fixed lookback from the current tip rather than resuming an exact cursor —
// safe (BridgeHub.claimed + the pre-flight check below make re-observing an already-claimed deposit
// a no-op) but not efficient at scale. See the bridge POC plan §3/§8 for the accepted tradeoff.
import "dotenv/config";
import { ethers } from "ethers";
import { loadConfig } from "./config.js";
import { watchDeposits, type DepositEvent } from "./watcher.js";
import { claimDeposit, TerminalClaimError } from "./claim.js";
import { ClaimTracker } from "./pending.js";

const LOOKBACK_BLOCKS = 600; // matches asc-message-relayer's DEFAULT_SCAN_LOOKBACK_BLOCKS convention

const BRIDGE_HUB_CLAIMED_ABI = [
  "function claimed(bytes32 key) view returns (bool)",
];

function claimedKey(chainKey: number, vault: string, nonce: bigint): string {
  // Must match BridgeHub._claimOne's key = keccak256(abi.encode(sourceChainKey, vault, nonce)) exactly.
  return ethers.keccak256(
    ethers.AbiCoder.defaultAbiCoder().encode(
      ["uint32", "address", "uint256"],
      [chainKey, vault, nonce],
    ),
  );
}

async function main() {
  const config = loadConfig();
  const ccProvider = new ethers.JsonRpcProvider(config.creditcoinRpcUrl);
  const claimSigner = new ethers.Wallet(config.claimSignerKey, ccProvider);
  const hub = new ethers.Contract(
    config.bridgeHubAddress,
    BRIDGE_HUB_CLAIMED_ABI,
    ccProvider,
  );
  const tracker = new ClaimTracker();

  console.log(
    `bridge-claimer-bot: signer ${claimSigner.address}, ${config.routes.length} route(s), hub ${config.bridgeHubAddress}`,
  );

  const stopFns = await Promise.all(
    config.routes.map(async (route) => {
      const spokeProvider = new ethers.JsonRpcProvider(route.rpcUrl);
      const tip = await spokeProvider.getBlockNumber();
      const fromBlock = Math.max(0, tip - LOOKBACK_BLOCKS);
      console.log(
        `  route chainKey=${route.chainKey}: vault=${route.bridgeVaultAddress} from block ${fromBlock}`,
      );

      return watchDeposits(
        spokeProvider,
        route.chainKey,
        route.bridgeVaultAddress,
        fromBlock,
        route.confirmationDepth,
        config.pollIntervalMs,
        (event) =>
          handleDeposit(
            event,
            route.bridgeVaultAddress,
            hub,
            tracker,
            ccProvider,
            claimSigner,
            config,
          ),
      );
    }),
  );

  process.on("SIGINT", () => {
    console.log("shutting down bridge-claimer-bot...");
    for (const stop of stopFns) stop();
    process.exit(0);
  });
}

async function handleDeposit(
  event: DepositEvent,
  vaultAddress: string,
  hub: ethers.Contract,
  tracker: ClaimTracker,
  ccProvider: ethers.JsonRpcProvider,
  claimSigner: ethers.Wallet,
  config: ReturnType<typeof loadConfig>,
): Promise<void> {
  const { chainKey, nonce } = event;
  if (tracker.shouldSkip(chainKey, nonce)) return;

  // Pre-flight view call: avoids a wasted proof fetch if a racing submitter (a user, or another bot
  // instance) already claimed this deposit — BridgeHub.claim itself would also just no-op-revert on
  // an all-already-claimed proof, but that only happens after the (much more expensive) proof fetch.
  const key = claimedKey(chainKey, vaultAddress, nonce);
  if (
    await (
      hub as unknown as { claimed: (k: string) => Promise<boolean> }
    ).claimed(key)
  ) {
    tracker.recordSuccess(chainKey, nonce);
    return;
  }

  tracker.markInFlight(chainKey, nonce);
  console.log(
    `deposit chainKey=${chainKey} nonce=${nonce} tx=${event.txHash}: claiming...`,
  );
  try {
    const result = await claimDeposit({
      ccProvider,
      bridgeHubAddress: config.bridgeHubAddress,
      claimSigner,
      proofGenUrl: config.proofGenUrl,
      sourceChainKey: chainKey,
      depositTxHash: event.txHash,
      depositBlockNumber: event.blockNumber,
    });
    tracker.recordSuccess(chainKey, nonce);
    console.log(
      `  ✅ claimed nonce=${nonce}: BridgeHub.claim tx ${result.txHash}`,
    );
  } catch (err) {
    if (err instanceof TerminalClaimError) {
      tracker.recordTerminal(chainKey, nonce);
      console.log(`  dropping nonce=${nonce} (${err.message})`);
    } else {
      tracker.recordTransientFailure(chainKey, nonce);
      console.error(
        `  transient failure claiming nonce=${nonce}, will retry:`,
        err,
      );
    }
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
