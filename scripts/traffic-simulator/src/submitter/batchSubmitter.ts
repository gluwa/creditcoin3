/**
 * Batch proof submitter
 *
 * Handles submission of multiple proofs in batch mode
 * using the native batch precompile with shared continuity proofs.
 */

import { ethers } from 'ethers';
import type { SimulatorConfig, TxInfo } from '../types.ts';
import { convertProofFormat, fetchProofForTx, submitBatchToPrecompile, isContinuityMismatchError } from './proofUtils.ts';
import { describeQueryMode } from '../query/queryFactory.ts';
import { submitProofsIndividually } from './singleSubmitter.ts';

/**
 * Submit proofs in batch mode
 *
 * Uses the native batch precompile with a shared continuity proof per block.
 * Falls back to single submissions if a batch fails.
 */
export async function submitBatchProofs(
  config: SimulatorConfig,
  txInfos: TxInfo[],
): Promise<{ successful: number; failed: number; batches: number }> {
  console.log(`📦 Batch submitting ${txInfos.length} proofs...`);
  console.log(`   Query mode: ${describeQueryMode(config.queryMode)}`);

  let successful = 0;
  let failed = 0;
  let batches = 0;

  const maxBatchSize = Math.min(config.batchSize, 10);

  // Group by block for potential optimization
  const byBlock = new Map<number, TxInfo[]>();
  for (const txInfo of txInfos) {
    const existing = byBlock.get(txInfo.blockNumber) || [];
    existing.push(txInfo);
    byBlock.set(txInfo.blockNumber, existing);
  }

  // Process each block's transactions
  for (const [blockNumber, blockTxs] of byBlock) {
    console.log(`  📦 Block ${blockNumber}: ${blockTxs.length} transactions`);

    for (let start = 0; start < blockTxs.length; start += maxBatchSize) {
      const batch = blockTxs.slice(start, start + maxBatchSize);
      const label = `block ${blockNumber}, batch ${Math.floor(start / maxBatchSize) + 1}`;
      const maxContinuityRetries = 2;
      const continuityRetryDelayMs = 15_000;

      let batchSucceeded = false;
      for (let attempt = 0; attempt <= maxContinuityRetries; attempt++) {
        try {
          const proofInputs = await Promise.all(
            batch.map(async (txInfo) => {
              const apiProof = await fetchProofForTx(
                config.proofApiUrl,
                config.chainKey,
                txInfo,
              );
              const proof = convertProofFormat(apiProof);
              if (!apiProof.txBytes) {
                throw new Error('Transaction bytes not found in API response');
              }
              const txBytes = ethers.getBytes(apiProof.txBytes);
              const headerNumber = apiProof.headerNumber ?? txInfo.blockNumber;
              if (headerNumber !== txInfo.blockNumber) {
                console.warn(
                  `⚠️  Proof header mismatch for ${txInfo.txHash.slice(0, 10)}...: expected ${txInfo.blockNumber}, got ${headerNumber}`,
                );
              }
              return {
                txInfo,
                proof,
                txBytes,
                headerNumber,
              };
            }),
          );

          const continuityProof = proofInputs[0].proof.continuityProof;
          const continuityMismatch = proofInputs.some(
            (input) =>
              input.proof.continuityProof.lowerEndpointDigest !== continuityProof.lowerEndpointDigest ||
              input.proof.continuityProof.roots.length !== continuityProof.roots.length ||
              input.proof.continuityProof.roots.some(
                (root, idx) => root !== continuityProof.roots[idx],
              ),
          );

          if (continuityMismatch) {
            console.warn(`⚠️  Continuity proof mismatch in ${label}, falling back to singles`);
            const fallback = await submitProofsIndividually(config, batch);
            successful += fallback.successful;
            failed += fallback.failed;
            batchSucceeded = true;
            break;
          }

          const heights = proofInputs.map((input) => input.headerNumber);
          const txBytesList = proofInputs.map((input) => input.txBytes);
          const merkleProofs = proofInputs.map((input) => input.proof.merkleProof);

          batches++;
          const batchResult = await submitBatchToPrecompile(
            config.cc3HttpUrl,
            config.cc3PrivateKey,
            config.chainKey,
            heights,
            txBytesList,
            merkleProofs,
            continuityProof,
          );

          console.log(
            `    ✅ Batch submitted (${label}): ${batch.length} proofs (gas: ${batchResult.gasUsed})`,
          );
          successful += batch.length;
          batchSucceeded = true;
          break;
        } catch (error) {
          if (isContinuityMismatchError(error) && attempt < maxContinuityRetries) {
            console.warn(
              `⚠️  Continuity mismatch in ${label}, retrying in ${continuityRetryDelayMs / 1000}s...`,
            );
            await new Promise((resolve) => setTimeout(resolve, continuityRetryDelayMs));
            continue;
          }
          const errorMsg = error instanceof Error ? error.message : String(error);
          console.error(`    ❌ Batch failed (${label}): ${errorMsg}`);
          const fallback = await submitProofsIndividually(config, batch);
          successful += fallback.successful;
          failed += fallback.failed;
          batchSucceeded = true;
          break;
        }
      }

      if (!batchSucceeded) {
        const fallback = await submitProofsIndividually(config, batch);
        successful += fallback.successful;
        failed += fallback.failed;
      }
    }
  }

  console.log(`📦 Batch complete: ${successful} successful, ${failed} failed`);

  return { successful, failed, batches };
}
