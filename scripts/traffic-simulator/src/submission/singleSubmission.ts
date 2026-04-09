/**
 * Single proof submitter
 *
 * Handles submission of individual proofs to the precompile.
 */

import type { SimulatorConfig, TxInfo } from "../types.ts";
import { fetchAndSubmitProof } from "./proofApi.ts";
import { withContinuityRetry } from "../utils/retry.ts";
import { sleep } from "../utils/sleep.ts";

/**
 * Submit a single proof for a transaction
 */
export async function submitSingleProof(
  config: SimulatorConfig,
  txInfo: TxInfo,
): Promise<{ success: boolean; error?: string }> {
  const label = txInfo.txHash.slice(0, 10);

  try {
    console.log(
      `📤 Submitting single proof for tx ${label}... (block ${txInfo.blockNumber}, index ${txInfo.txIndex})`,
    );

    const result = await withContinuityRetry(
      () =>
        fetchAndSubmitProof(
          config.proofApiUrl,
          config.cc3HttpUrl,
          config.cc3PrivateKey,
          config.chainKey,
          txInfo,
        ),
      label,
    );

    console.log(
      `✅ Proof submitted: ${
        result.txHash.slice(0, 10)
      }... (gas: ${result.gasUsed})`,
    );
    return { success: true };
  } catch (error) {
    const errorMsg = error instanceof Error ? error.message : String(error);
    console.error(`❌ Failed to submit proof for ${label}...: ${errorMsg}`);
    return { success: false, error: errorMsg };
  }
}

/**
 * Submit multiple proofs individually (not as a batch)
 */
export async function submitProofsIndividually(
  config: SimulatorConfig,
  txInfos: TxInfo[],
  delayMs = 1000,
  onError?: (error: string) => void,
): Promise<{ successful: number; failed: number }> {
  let successful = 0;
  let failed = 0;

  for (let i = 0; i < txInfos.length; i++) {
    const result = await submitSingleProof(config, txInfos[i]);

    if (result.success) {
      successful++;
    } else {
      failed++;
      if (onError && result.error) {
        onError(result.error);
      }
    }

    // Add delay between submissions (except for the last one)
    if (i < txInfos.length - 1 && delayMs > 0) {
      await sleep(delayMs);
    }
  }

  return { successful, failed };
}
