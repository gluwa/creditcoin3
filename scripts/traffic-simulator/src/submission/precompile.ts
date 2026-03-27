/**
 * Precompile interaction — simulation, sending, receipt handling, and encoding.
 */

import { ethers, JsonRpcProvider } from "ethers";
import type { FormattedProof } from "../types.ts";
import {
  BATCH_PROOF_GAS_LIMIT,
  BATCH_VERIFY_SIG,
  MAX_TRANSIENT_RETRIES,
  PRECOMPILE_ADDRESS,
  RECEIPT_TIMEOUT_MS,
  RPC_TIMEOUT_MS,
  SINGLE_PROOF_GAS_LIMIT,
  SINGLE_VERIFY_SIG,
  TRANSIENT_RETRY_BASE_DELAY_MS,
} from "../constants.ts";
import {
  decodeRevertMessage,
  getErrorMessage,
  isNonceError,
  isReplacementUnderpricedError,
  isTransientNetworkError,
} from "../utils/errors.ts";
import { sleep } from "../utils/sleep.ts";
import { withTimeout } from "../utils/retry.ts";
import {
  getSigner,
  resetSigner,
  type SignerEntry,
  withSubmissionLock,
} from "./signer.ts";
import { bumpFees, getFeeOverrides } from "./fees.ts";
import PRECOMPILE_ABI from "../abi/block_prover.json" with { type: "json" };

// ============================================================================
// Transaction Submission (private)
// ============================================================================

async function sendTransaction(
  signer: ethers.NonceManager,
  provider: JsonRpcProvider,
  request: ethers.TransactionRequest,
  label: string,
): Promise<ethers.TransactionResponse> {
  try {
    return await signer.sendTransaction(request);
  } catch (error) {
    if (!isReplacementUnderpricedError(error)) throw error;

    const address = await signer.getAddress();
    const nonce = await withTimeout(
      provider.getTransactionCount(address, "pending"),
      RPC_TIMEOUT_MS,
      `${label} nonce`,
    );
    console.debug("Retrying underpriced tx", { label, nonce });
    return await signer.sendTransaction({
      ...request,
      nonce,
      ...bumpFees(request),
    });
  }
}

async function waitForReceipt(
  provider: JsonRpcProvider,
  tx: ethers.TransactionResponse,
  label: string,
): Promise<ethers.TransactionReceipt> {
  try {
    const receipt = await withTimeout(tx.wait(), RECEIPT_TIMEOUT_MS, label);
    if (!receipt) throw new Error(`${label} returned empty receipt`);
    return receipt;
  } catch (error) {
    // Handle replaced transactions
    const err = error as { code?: string; receipt?: ethers.TransactionReceipt };
    if (err.code === "TRANSACTION_REPLACED" && err.receipt) {
      console.debug("Transaction replaced", {
        label,
        original: tx.hash,
        replacement: err.receipt.hash,
      });
      return err.receipt;
    }

    // Try direct lookup as fallback
    const receipt = await provider.getTransactionReceipt(tx.hash).catch(() =>
      null
    );
    if (receipt) return receipt;

    throw error;
  }
}

// ============================================================================
// Precompile Submission (Unified)
// ============================================================================

interface PrecompileParams {
  cc3HttpUrl: string;
  privateKey: string;
  chainKey: number;
  data: string;
  gasLimit: bigint;
  label: string;
}

async function executePrecompileCall(
  params: PrecompileParams,
): Promise<{ txHash: string; gasUsed: bigint }> {
  const { cc3HttpUrl, privateKey, data, gasLimit, label } = params;

  /**
   * Simulate the transaction with retry for transient network errors.
   * This is safe to retry because no nonce is used during simulation.
   * Returns the entry (which may be refreshed) along with the fee overrides.
   */
  const simulateWithRetry = async (
    initialEntry: SignerEntry,
  ): Promise<
    { entry: SignerEntry; feeOverrides: ethers.TransactionRequest }
  > => {
    let entry = initialEntry;

    for (let attempt = 0; attempt <= MAX_TRANSIENT_RETRIES; attempt++) {
      // Destructure inside the loop to use fresh references after reset
      const { signer, provider } = entry;

      try {
        const from = await signer.getAddress();

        // Simulate the transaction
        await withTimeout(
          provider.call({ to: PRECOMPILE_ADDRESS, data, from }),
          RPC_TIMEOUT_MS,
          `${label} simulation`,
        );

        // Get fee data
        const feeOverrides = await getFeeOverrides(provider);
        console.debug(`${label} fees`, { ...feeOverrides });

        return { entry, feeOverrides };
      } catch (error) {
        // Check for revert errors (not retriable)
        const revertData = (error as { data?: string }).data;
        if (revertData) {
          const revertMsg = decodeRevertMessage(revertData);
          throw new Error(`${label} will revert: ${revertMsg}`);
        }

        // Check if it's a transient network error that should be retried
        if (
          isTransientNetworkError(error) && attempt < MAX_TRANSIENT_RETRIES
        ) {
          const delayMs = TRANSIENT_RETRY_BASE_DELAY_MS *
              Math.pow(2, attempt) +
            Math.random() * 500;
          console.warn(
            `⚠️  ${label}: transient network error during simulation (${
              getErrorMessage(error)
            }), retrying in ${Math.round(delayMs / 1000)}s (${
              attempt + 1
            }/${MAX_TRANSIENT_RETRIES})...`,
          );
          // Reset the provider and update entry for next iteration
          entry = resetSigner(cc3HttpUrl, privateKey);
          await sleep(delayMs);
          continue;
        }

        throw new Error(
          `${label} simulation failed: ${getErrorMessage(error)}`,
        );
      }
    }

    // Should never reach here
    throw new Error(
      `${label} simulation failed after ${MAX_TRANSIENT_RETRIES} retries`,
    );
  };

  /**
   * Execute the full submission (simulate + send).
   * Transient retries only happen during simulation, not after tx is sent.
   */
  const execute = async (initialEntry: SignerEntry) => {
    // Simulation phase with transient retry (safe - no nonce used)
    // Use the returned entry in case it was refreshed during retry
    const { entry, feeOverrides } = await simulateWithRetry(initialEntry);
    const { signer, provider } = entry;

    // Send transaction phase (no transient retry - nonce is used)
    // If this fails, we rely on nonce error handling, not blind retry
    const tx = await sendTransaction(
      signer,
      provider,
      { to: PRECOMPILE_ADDRESS, data, gasLimit, ...feeOverrides },
      label,
    );
    console.debug(`${label} sent`, { txHash: tx.hash, nonce: tx.nonce });

    return { tx, provider };
  };

  // Execute with submission lock and nonce retry
  const { tx, provider } = await withSubmissionLock(async () => {
    try {
      return await execute(getSigner(cc3HttpUrl, privateKey));
    } catch (error) {
      if (isNonceError(error)) {
        console.warn(`⚠️  ${label}: nonce out of sync, retrying...`);
        return await execute(resetSigner(cc3HttpUrl, privateKey));
      }
      throw error;
    }
  });

  const receipt = await waitForReceipt(provider, tx, `${label} confirmation`);
  if (receipt.status !== 1) throw new Error(`${label} reverted`);

  console.debug(`${label} confirmed`, {
    txHash: receipt.hash,
    gasUsed: receipt.gasUsed,
  });
  return { txHash: receipt.hash, gasUsed: receipt.gasUsed };
}

// ============================================================================
// Proof Encoding (private)
// ============================================================================

function encodeMerkleProof(proof: FormattedProof["merkleProof"]) {
  return [proof.root, proof.siblings.map((s) => [s.hash, s.isLeft])];
}

function encodeContinuityProof(proof: FormattedProof["continuityProof"]) {
  return [proof.lowerEndpointDigest, proof.roots];
}

// ============================================================================
// Public Submission Functions
// ============================================================================

export async function submitSingleToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  blockHeight: number,
  txBytes: Uint8Array,
  proof: FormattedProof,
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  const iface = new ethers.Interface(PRECOMPILE_ABI);
  const func = iface.getFunction(SINGLE_VERIFY_SIG);
  if (!func) throw new Error("Single verifyAndEmit not found in ABI");

  const data = iface.encodeFunctionData(func, [
    BigInt(chainKey),
    BigInt(blockHeight),
    ethers.hexlify(txBytes),
    encodeMerkleProof(proof.merkleProof),
    encodeContinuityProof(proof.continuityProof),
  ]);

  console.debug("Submitting single proof", {
    chainKey,
    blockHeight,
    txBytesLen: txBytes.length,
  });

  const result = await executePrecompileCall({
    cc3HttpUrl,
    privateKey,
    chainKey,
    data,
    gasLimit: SINGLE_PROOF_GAS_LIMIT,
    label: "Single submit",
  });

  return { success: true, ...result };
}

export async function submitBatchToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  heights: number[],
  txBytesList: Uint8Array[],
  merkleProofs: FormattedProof["merkleProof"][],
  continuityProof: FormattedProof["continuityProof"],
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  const iface = new ethers.Interface(PRECOMPILE_ABI);
  const func = iface.getFunction(BATCH_VERIFY_SIG);
  if (!func) throw new Error("Batch verifyAndEmit not found in ABI");

  const data = iface.encodeFunctionData(func, [
    BigInt(chainKey),
    heights.map(BigInt),
    txBytesList.map((b) => ethers.hexlify(b)),
    merkleProofs.map(encodeMerkleProof),
    encodeContinuityProof(continuityProof),
  ]);

  console.debug("Submitting batch", { chainKey, batchSize: heights.length });

  const result = await executePrecompileCall({
    cc3HttpUrl,
    privateKey,
    chainKey,
    data,
    gasLimit: BATCH_PROOF_GAS_LIMIT,
    label: "Batch submit",
  });

  return { success: true, ...result };
}
