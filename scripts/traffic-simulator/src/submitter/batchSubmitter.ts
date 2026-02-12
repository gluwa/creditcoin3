/**
 * Batch proof submitter
 *
 * Submits multiple proofs in batch mode using the native batch precompile.
 * Continuity merging mirrors scripts/SubmitBatchProof.js:
 * - gather per-query continuity roots
 * - build one shared continuity proof from min->max query height
 * - submit verifyAndEmit batch with shared continuity
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

interface ContinuityProof {
  lowerEndpointDigest: string;
  roots: string[];
}

interface BatchPayload {
  heights: number[];
  txBytesList: Uint8Array[];
  merkleProofs: Array<
    { root: string; siblings: Array<{ hash: string; isLeft: boolean }> }
  >;
  continuityProof: ContinuityProof;
}

interface ProofInput {
  txInfo: TxInfo;
  proof: {
    continuityProof: ContinuityProof;
    merkleProof: {
      root: string;
      siblings: Array<{ hash: string; isLeft: boolean }>;
    };
  };
  txBytes: Uint8Array;
  headerNumber: number;
}

function sortProofInputs(inputs: ProofInput[]): void {
  inputs.sort((a, b) => {
    if (a.headerNumber !== b.headerNumber) {
      return a.headerNumber - b.headerNumber;
    }
    return a.txInfo.txIndex - b.txInfo.txIndex;
  });
}

/** Merge roots from all proofs into one shared continuity chain. */
function buildSharedContinuityProof(
  batchInputs: Array<
    { headerNumber: number; proof: { continuityProof: ContinuityProof } }
  >,
): ContinuityProof | null {
  if (batchInputs.length === 0) return null;

  const heights = batchInputs.map((b) => b.headerNumber);
  const minHeight = Math.min(...heights);
  const maxHeight = Math.max(...heights);

  const heightToRoot = new Map<number, string>();
  const heightToLowerDigest = new Map<number, string>();

  for (const { headerNumber, proof } of batchInputs) {
    const cp = proof.continuityProof;
    if (!cp?.roots?.length) continue;

    heightToLowerDigest.set(headerNumber, cp.lowerEndpointDigest);

    for (let i = 0; i < cp.roots.length; i++) {
      const blockHeight = headerNumber + i;
      if (!heightToRoot.has(blockHeight)) {
        heightToRoot.set(blockHeight, cp.roots[i]);
      }
    }
  }

  const lowerEndpointDigest = heightToLowerDigest.get(minHeight);
  if (!lowerEndpointDigest) return null;

  const allHeights = [...heightToRoot.keys()].sort((a, b) => a - b);
  const actualMaxHeight = allHeights[allHeights.length - 1];

  if (actualMaxHeight < maxHeight) return null;

  const roots: string[] = [];
  for (let h = minHeight; h <= actualMaxHeight; h++) {
    const root = heightToRoot.get(h);
    if (!root) return null;
    roots.push(root);
  }

  return { lowerEndpointDigest, roots };
}

/** Validate payload before submitting to batch verifyAndEmit. */
function validateBatchPayload(
  payload: BatchPayload,
  startHeight: number,
): string | null {
  const { heights, merkleProofs, continuityProof } = payload;
  const maxHeight = Math.max(...heights);

  // roots[0] maps to startHeight; chain must cover all query heights.
  const lastContinuityBlock = startHeight + continuityProof.roots.length - 1;
  if (lastContinuityBlock < maxHeight) {
    return `Continuity chain ends at ${lastContinuityBlock}, does not cover max query height ${maxHeight}`;
  }

  // Each merkle root must match continuity root at that height offset.
  for (let i = 0; i < heights.length; i++) {
    const offset = heights[i] - startHeight;
    const expectedRoot = continuityProof.roots[offset];
    const actualRoot = merkleProofs[i].root;
    if (expectedRoot !== actualRoot) {
      return `Merkle root mismatch at height ${heights[i]}: expected ${
        expectedRoot.slice(0, 10)
      }..., got ${actualRoot.slice(0, 10)}...`;
    }
  }

  return null;
}

async function fetchProofInputForTx(
  config: SimulatorConfig,
  txInfo: TxInfo,
): Promise<ProofInput> {
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
}

/** Submit proofs in batch mode. Falls back to single submissions on failure. */
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
    txInfos.map((txInfo) => fetchProofInputForTx(config, txInfo)),
  );
  sortProofInputs(proofInputs);

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
      const batchTxInfos = batchInputs.map((input) => input.txInfo);

      // Refetch on each continuity retry attempt; stale proofs often fail again.
      const submitFreshBatch = async () => {
        const freshInputs = await Promise.all(
          batchTxInfos.map((txInfo) => fetchProofInputForTx(config, txInfo)),
        );
        sortProofInputs(freshInputs);

        const heights = freshInputs.map((input) => input.headerNumber);
        const txBytesList = freshInputs.map((input) => input.txBytes);
        const merkleProofs = freshInputs.map((input) => input.proof.merkleProof);
        const sharedContinuityProof = buildSharedContinuityProof(freshInputs);
        if (!sharedContinuityProof) {
          throw new Error("Could not build shared continuity proof from fresh inputs");
        }

        const payload: BatchPayload = {
          heights,
          txBytesList,
          merkleProofs,
          continuityProof: sharedContinuityProof,
        };
        const validationError = validateBatchPayload(
          payload,
          Math.min(...heights),
        );
        if (validationError) {
          throw new Error(`Batch validation failed: ${validationError}`);
        }

        return submitBatchToPrecompile(
          config.cc3HttpUrl,
          config.cc3PrivateKey,
          config.chainKey,
          heights,
          txBytesList,
          merkleProofs,
          sharedContinuityProof,
        );
      };

      console.log(`    📦 Batch txs: ${batchTxHashes}`);
      const batchResult = await withContinuityRetry(
        submitFreshBatch,
        label,
      );
      batches++;

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
