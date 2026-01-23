/**
 * Batch proof submitter
 *
 * Handles submission of multiple proofs in batch mode
 * using the native batch precompile with shared continuity proofs.
 */

import { ethers } from "ethers";
import type { SimulatorConfig, TxInfo } from "../types.ts";
import {
  convertProofFormat,
  fetchProofForTx,
  submitBatchToPrecompile,
} from "./proofUtils.ts";
import { submitProofsIndividually } from "./singleSubmitter.ts";
import { withContinuityRetry } from "../utils/retry.ts";
import { randomInt } from "../utils/random.ts";

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
  const uniqueBlocks = new Set(txInfos.map((tx) => tx.blockNumber)).size;
  console.log(
    `📦 Batch submitting ${txInfos.length} proofs across ${uniqueBlocks} blocks...`,
  );

  let successful = 0;
  let failed = 0;
  let batches = 0;

  const maxBatchSize = Math.min(config.batchSize, 10);
  const proofInputs = await Promise.all(
    txInfos.map(async (txInfo) => {
      const apiProof = await fetchProofForTx(
        config.proofApiUrl,
        config.chainKey,
        txInfo,
      );
      const proof = convertProofFormat(apiProof);
      if (!apiProof.txBytes) {
        throw new Error("Transaction bytes not found in API response");
      }
      const txBytes = ethers.getBytes(apiProof.txBytes);
      const headerNumber = apiProof.headerNumber ?? txInfo.blockNumber;
      if (headerNumber !== txInfo.blockNumber) {
        console.warn(
          `⚠️  Proof header mismatch for ${
            txInfo.txHash.slice(0, 10)
          }...: expected ${txInfo.blockNumber}, got ${headerNumber}`,
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

  proofInputs.sort((a, b) => {
    if (a.headerNumber !== b.headerNumber) {
      return a.headerNumber - b.headerNumber;
    }
    return a.txInfo.txIndex - b.txInfo.txIndex;
  });

  let index = 0;
  while (index < proofInputs.length) {
    const base = proofInputs[index];
    const continuityProof = base.proof.continuityProof;
    if (continuityProof.roots.length === 0) {
      console.warn(
        `⚠️  Empty continuity proof for block ${base.headerNumber}, falling back to single`,
      );
      const fallback = await submitProofsIndividually(config, [base.txInfo]);
      successful += fallback.successful;
      failed += fallback.failed;
      index += 1;
      continue;
    }
    const startHeight = base.headerNumber;
    const lastHeight = startHeight + continuityProof.roots.length - 1;
    const targetBatchSize = randomInt(2, maxBatchSize);
    const batchInputs = [base];

    let nextIndex = index + 1;
    while (
      nextIndex < proofInputs.length && batchInputs.length < targetBatchSize
    ) {
      const candidate = proofInputs[nextIndex];
      if (candidate.headerNumber > lastHeight) {
        break;
      }
      const offset = candidate.headerNumber - startHeight;
      const expectedRoot = continuityProof.roots[offset];
      if (expectedRoot !== candidate.proof.merkleProof.root) {
        break;
      }
      batchInputs.push(candidate);
      nextIndex++;
    }

    // If only 1 proof, submit as single instead of batch
    if (batchInputs.length === 1) {
      const result = await submitProofsIndividually(config, [
        batchInputs[0].txInfo,
      ]);
      successful += result.successful;
      failed += result.failed;
      index = nextIndex;
      continue;
    }

    const label = `blocks ${batchInputs[0].headerNumber}-${
      batchInputs[batchInputs.length - 1].headerNumber
    }`;
    const batchTxHashes = batchInputs
      .map((input) => `${input.txInfo.txHash.slice(0, 10)}...`)
      .join(", ");

    try {
      const heights = batchInputs.map((input) => input.headerNumber);
      const txBytesList = batchInputs.map((input) => input.txBytes);
      const merkleProofs = batchInputs.map((input) => input.proof.merkleProof);

      batches++;
      console.log(`    📦 Batch txs: ${batchTxHashes}`);
      const batchResult = await withContinuityRetry(
        () =>
          submitBatchToPrecompile(
            config.cc3HttpUrl,
            config.cc3PrivateKey,
            config.chainKey,
            heights,
            txBytesList,
            merkleProofs,
            continuityProof,
          ),
        label,
      );

      console.log(
        `    ✅ Batch submitted (${label}): ${batchInputs.length} proofs (tx: ${
          batchResult.txHash.slice(0, 10)
        }..., gas: ${batchResult.gasUsed})`,
      );
      successful += batchInputs.length;
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      console.error(`    ❌ Batch failed (${label}): ${errorMsg}`);
      const fallback = await submitProofsIndividually(
        config,
        batchInputs.map((input) => input.txInfo),
      );
      successful += fallback.successful;
      failed += fallback.failed;
    }

    index = nextIndex;
  }

  console.log(`📦 Batch complete: ${successful} successful, ${failed} failed`);

  return { successful, failed, batches };
}
