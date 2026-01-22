/**
 * Single proof submitter
 *
 * Handles submission of individual proofs to the precompile.
 */

import type { SimulatorConfig, TxInfo } from '../types.ts';
import { fetchAndSubmitProof, isContinuityMismatchError } from './proofUtils.ts';

/**
 * Submit a single proof for a transaction
 */
export async function submitSingleProof(
  config: SimulatorConfig,
  txInfo: TxInfo,
): Promise<{ success: boolean; error?: string }> {
  const maxContinuityRetries = 2;
  const continuityRetryDelayMs = 15_000;

  for (let attempt = 0; attempt <= maxContinuityRetries; attempt++) {
    try {
      console.log(
        `📤 Submitting single proof for tx ${
          txInfo.txHash.slice(0, 10)
        }... (block ${txInfo.blockNumber}, index ${txInfo.txIndex})`,
      );

      const result = await fetchAndSubmitProof(
        config.proofApiUrl,
        config.cc3HttpUrl,
        config.cc3PrivateKey,
        config.chainKey,
        txInfo,
      );

      console.log(`✅ Proof submitted: ${result.txHash.slice(0, 10)}... (gas: ${result.gasUsed})`);

      return { success: true };
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      if (isContinuityMismatchError(error) && attempt < maxContinuityRetries) {
        console.warn(
          `⚠️  Continuity mismatch for ${txInfo.txHash.slice(0, 10)}..., retrying in ${
            continuityRetryDelayMs / 1000
          }s...`,
        );
        await new Promise((resolve) => setTimeout(resolve, continuityRetryDelayMs));
        continue;
      }
      console.error(`❌ Failed to submit proof for ${txInfo.txHash.slice(0, 10)}...: ${errorMsg}`);
      return { success: false, error: errorMsg };
    }
  }

  return { success: false, error: 'Unknown error' };
}

/**
 * Submit multiple proofs individually (not as a batch)
 */
export async function submitProofsIndividually(
  config: SimulatorConfig,
  txInfos: TxInfo[],
  delayMs = 1000,
): Promise<{ successful: number; failed: number }> {
  let successful = 0;
  let failed = 0;

  for (let i = 0; i < txInfos.length; i++) {
    const txInfo = txInfos[i];

    const result = await submitSingleProof(config, txInfo);

    if (result.success) {
      successful++;
    } else {
      failed++;
    }

    // Add delay between submissions (except for the last one)
    if (i < txInfos.length - 1 && delayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }

  return { successful, failed };
}
