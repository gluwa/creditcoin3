// Core proof-fetch + submit logic for one deposit, plus a one-shot CLI entry point for the manual
// round-trip dry run: `tsx claim.ts <sourceChainKey> <depositTxHash>`.
//
// Proof assembly mirrors scripts/claim-delivery.mts's already-proven encoding for the same
// IASCProofVerifier/BlockProverTypes machinery (that script proves a delivery tx for the relayer's
// fee-claim path; this proves a deposit tx for BridgeHub instead) — same InclusionProof/
// ContinuityProof shapes, just a different destination contract and call.
import { ethers } from "ethers";
import { proofProvider } from "@gluwa/usc-sdk";
import { loadConfig } from "./config.js";

const BRIDGE_HUB_ABI = [
  "function claim(uint32 sourceChainKey, uint64 blockHeight, (uint8 kind, bytes32 root, bytes data) inclusionProof, (bytes32 lowerEndpointDigest, bytes32[] roots) continuityProof) external",
  "function claimed(bytes32 key) view returns (bool)",
  "error AllDepositsAlreadyClaimed()",
  "error WrongEmitter(address got, address expected)",
  "error ChainNotConfigured(uint32 chainKey)",
];

// Terminal reverts will never succeed on retry — the bot should drop the deposit, not back off and
// retry. Anything else (RPC error, nonce race, proof-gen not-ready, or a genuine on-chain bug we'd
// rather keep retrying than silently drop) is treated as transient.
const TERMINAL_ERRORS = new Set([
  "AllDepositsAlreadyClaimed",
  "WrongEmitter",
  "ChainNotConfigured",
]);

export class TerminalClaimError extends Error {
  constructor(reason: string) {
    super(`terminal: ${reason}`);
  }
}

export interface ClaimDepositArgs {
  ccProvider: ethers.JsonRpcProvider;
  bridgeHubAddress: string;
  claimSigner: ethers.Wallet;
  proofGenUrl: string;
  sourceChainKey: number;
  depositTxHash: string;
  depositBlockNumber: number;
}

export interface ClaimResult {
  txHash: string;
  blockNumber: number;
}

/** Fetches the native USC proof for `depositTxHash` and submits `BridgeHub.claim`. Throws
 *  `TerminalClaimError` for a decoded revert that will never succeed on retry; any other throw
 *  (RPC error, proof-gen not ready, etc.) is transient and safe to retry later. */
export async function claimDeposit(
  args: ClaimDepositArgs,
): Promise<ClaimResult> {
  const hub = new ethers.Contract(
    args.bridgeHubAddress,
    BRIDGE_HUB_ABI,
    args.claimSigner,
  );

  const proofBuilder = new proofProvider.service.ProofBuilder(
    args.sourceChainKey,
    args.proofGenUrl,
  );
  console.log(
    `  waiting for chainKey=${args.sourceChainKey} height=${args.depositBlockNumber} to be attested...`,
  );
  await proofBuilder.waitUntilHeightAttested(
    args.sourceChainKey,
    args.depositBlockNumber,
  );

  const result = await proofBuilder.getProof(args.depositTxHash);
  if (!result.success || !result.data) {
    throw new Error(
      `proof-gen failed for ${args.depositTxHash}: ${result.error ?? "no data"}`,
    );
  }
  const proof = result.data;
  if (!proof.txBytes) {
    throw new Error(
      `proof-gen returned no txBytes for ${args.depositTxHash} yet (not fully indexed)`,
    );
  }

  const inclusionData = ethers.AbiCoder.defaultAbiCoder().encode(
    ["bytes", "tuple(bytes32 sibling, bool isLeft)[]"],
    [
      proof.txBytes,
      proof.merkleProof.siblings.map((s) => ({
        sibling: s.hash,
        isLeft: s.isLeft,
      })),
    ],
  );
  const inclusionProof = {
    kind: 0,
    root: proof.merkleProof.root,
    data: inclusionData,
  }; // ProofKind.BinaryMerkle = 0
  const continuityProof = {
    lowerEndpointDigest: proof.continuityProof.lowerEndpointDigest,
    roots: proof.continuityProof.roots,
  };

  try {
    const tx = await hub.claim(
      args.sourceChainKey,
      proof.headerNumber,
      inclusionProof,
      continuityProof,
    );
    const receipt = await tx.wait();
    return {
      txHash: receipt.hash as string,
      blockNumber: receipt.blockNumber as number,
    };
  } catch (err) {
    const parsed = tryParseHubError(hub, err);
    if (parsed && TERMINAL_ERRORS.has(parsed)) {
      throw new TerminalClaimError(parsed);
    }
    throw err;
  }
}

function tryParseHubError(
  hub: ethers.Contract,
  err: unknown,
): string | undefined {
  const data =
    (err as { data?: string; error?: { data?: string } })?.data ??
    (err as { error?: { data?: string } })?.error?.data;
  if (!data) return undefined;
  try {
    return hub.interface.parseError(data)?.name;
  } catch {
    return undefined;
  }
}

// One-shot CLI: tsx claim.ts <sourceChainKey> <depositTxHash>
// Looks up the deposit's block number itself (from the route configured for that chain key), then
// runs the same claimDeposit path the daemon (index.ts) uses per-event.
if (import.meta.url === `file://${process.argv[1]}`) {
  const [chainKeyArg, txHashArg] = process.argv.slice(2);
  if (!chainKeyArg || !txHashArg) {
    console.error("usage: tsx claim.ts <sourceChainKey> <depositTxHash>");
    process.exit(1);
  }
  const sourceChainKey = Number(chainKeyArg);

  const config = loadConfig();
  const route = config.routes.find((r) => r.chainKey === sourceChainKey);
  if (!route)
    throw new Error(
      `no configured route for chainKey ${sourceChainKey} in CLAIMER_ROUTES`,
    );

  const spokeProvider = new ethers.JsonRpcProvider(route.rpcUrl);
  const ccProvider = new ethers.JsonRpcProvider(config.creditcoinRpcUrl);
  const claimSigner = new ethers.Wallet(config.claimSignerKey, ccProvider);

  const receipt = await spokeProvider.getTransactionReceipt(txHashArg);
  if (!receipt)
    throw new Error(
      `deposit tx ${txHashArg} not found on chainKey ${sourceChainKey}`,
    );

  console.log(
    `claiming deposit ${txHashArg} (chainKey=${sourceChainKey}, block=${receipt.blockNumber})`,
  );
  const result = await claimDeposit({
    ccProvider,
    bridgeHubAddress: config.bridgeHubAddress,
    claimSigner,
    proofGenUrl: config.proofGenUrl,
    sourceChainKey,
    depositTxHash: txHashArg,
    depositBlockNumber: receipt.blockNumber,
  });
  console.log(
    `✅ claimed: BridgeHub.claim tx ${result.txHash} (block ${result.blockNumber})`,
  );
  process.exit(0);
}
