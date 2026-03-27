/**
 * Batch proof submitter
 *
 * Submits multiple proofs in batch mode using the native batch precompile.
 * Uses the proof-gen-api batch endpoint to fetch proofs in a single request,
 * which returns a shared continuity proof covering all queried blocks.
 */

import { ethers } from "ethers";
import type {
  BatchProofResponse,
  ProofQuery,
  SimulatorConfig,
  TxInfo,
} from "../types.ts";
import { fetchBatchProofWithRetry } from "./proofApi.ts";
import { submitBatchToPrecompile } from "./precompile.ts";
import { submitProofsIndividually } from "./singleSubmission.ts";
import { withContinuityRetry } from "../utils/retry.ts";

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
  merkleProof: {
    root: string;
    siblings: Array<{ hash: string; isLeft: boolean }>;
  };
  txBytes: Uint8Array;
  headerNumber: number;
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

/** Group TxInfo[] into ProofQuery format for the batch API. */
function groupIntoProofQueries(txInfos: TxInfo[]): ProofQuery[] {
  const byBlock = new Map<number, number[]>();
  for (const tx of txInfos) {
    const existing = byBlock.get(tx.blockNumber) ?? [];
    existing.push(tx.txIndex);
    byBlock.set(tx.blockNumber, existing);
  }
  return Array.from(byBlock.entries())
    .sort(([a], [b]) => a - b)
    .map(([headerNumber, txIndexes]) => ({ headerNumber, txIndexes }));
}

/**
 * Chunk ProofQueries to respect batch API limits:
 * max 10 queries per request, max 10 tx indexes per query.
 */
function chunkQueries(queries: ProofQuery[]): ProofQuery[][] {
  const chunks: ProofQuery[][] = [];
  let current: ProofQuery[] = [];

  for (const query of queries) {
    // Split query if it has too many tx indexes
    for (let i = 0; i < query.txIndexes.length; i += 10) {
      const subQuery: ProofQuery = {
        headerNumber: query.headerNumber,
        txIndexes: query.txIndexes.slice(i, i + 10),
      };

      if (current.length >= 10) {
        chunks.push(current);
        current = [];
      }
      current.push(subQuery);
    }
  }

  if (current.length > 0) {
    chunks.push(current);
  }
  return chunks;
}

/** Convert a batch API response into sorted ProofInput array. */
function parseBatchResponse(
  response: BatchProofResponse,
  txInfos: TxInfo[],
): ProofInput[] {
  const inputs: ProofInput[] = [];

  for (const txInfo of txInfos) {
    const blockProofs = response.merkleProofs[String(txInfo.blockNumber)];
    if (!blockProofs) {
      throw new Error(
        `Batch response missing block ${txInfo.blockNumber}`,
      );
    }

    const entry = blockProofs[String(txInfo.txIndex)];
    if (!entry) {
      throw new Error(
        `Batch response missing tx index ${txInfo.txIndex} for block ${txInfo.blockNumber}`,
      );
    }

    if (!entry.txBytes) {
      throw new Error(
        `Batch response missing txBytes for block ${txInfo.blockNumber} tx ${txInfo.txIndex}`,
      );
    }

    inputs.push({
      txInfo,
      merkleProof: entry.merkleProof,
      txBytes: ethers.getBytes(entry.txBytes),
      headerNumber: txInfo.blockNumber,
    });
  }

  // Sort by block number then tx index
  inputs.sort((a, b) => {
    if (a.headerNumber !== b.headerNumber) {
      return a.headerNumber - b.headerNumber;
    }
    return a.txInfo.txIndex - b.txInfo.txIndex;
  });

  return inputs;
}

/** Submit proofs in batch mode. Falls back to single submissions on failure. */
export async function submitBatchProofs(
  config: SimulatorConfig,
  txInfos: TxInfo[],
  onError?: (error: string) => void,
): Promise<{ successful: number; failed: number; batches: number }> {
  const uniqueBlocks = new Set(txInfos.map((tx) => tx.blockNumber)).size;
  console.log(
    `📦 Batch submitting ${txInfos.length} proofs across ${uniqueBlocks} blocks...`,
  );

  let successful = 0;
  let failed = 0;
  let batches = 0;

  const maxBatchSize = Math.min(config.batchSize, 10);
  const queries = groupIntoProofQueries(txInfos);
  const queryChunks = chunkQueries(queries);

  for (const chunk of queryChunks) {
    // Collect the TxInfos covered by this chunk
    const chunkBlockNumbers = new Set(chunk.map((q) => q.headerNumber));
    const chunkTxInfos = txInfos.filter((tx) =>
      chunkBlockNumbers.has(tx.blockNumber) &&
      chunk.some((q) =>
        q.headerNumber === tx.blockNumber &&
        q.txIndexes.includes(tx.txIndex)
      )
    );

    let batchResponse: BatchProofResponse;
    let proofInputs: ProofInput[];
    try {
      batchResponse = await fetchBatchProofWithRetry(
        config.proofApiUrl,
        config.chainKey,
        chunk,
      );
      proofInputs = parseBatchResponse(batchResponse, chunkTxInfos);
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      console.error(`    ❌ Batch fetch failed: ${errorMsg}`);
      onError?.(errorMsg);
      console.log(`    ↩️  Falling back to single submissions...`);
      const fallback = await submitProofsIndividually(
        config,
        chunkTxInfos,
        1000,
        onError,
      );
      successful += fallback.successful;
      failed += fallback.failed;
      continue;
    }

    const sharedContinuityProof = batchResponse.continuityProof;
    const startHeight = batchResponse.fromHeader;

    // Split proofInputs into precompile-sized batches
    let index = 0;
    while (index < proofInputs.length) {
      const targetBatchSize = maxBatchSize <= 2
        ? 2
        : Math.floor(Math.random() * (maxBatchSize - 2 + 1)) + 2;
      const batchInputs = proofInputs.slice(
        index,
        index + targetBatchSize,
      );

      // If only 1 proof, submit as single instead of batch
      if (batchInputs.length === 1) {
        const result = await submitProofsIndividually(
          config,
          [batchInputs[0].txInfo],
          1000,
          onError,
        );
        successful += result.successful;
        failed += result.failed;
        index += 1;
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
          const freshQueries = groupIntoProofQueries(batchTxInfos);
          const freshResponse = await fetchBatchProofWithRetry(
            config.proofApiUrl,
            config.chainKey,
            freshQueries,
          );
          const freshInputs = parseBatchResponse(freshResponse, batchTxInfos);

          const heights = freshInputs.map((input) => input.headerNumber);
          const txBytesList = freshInputs.map((input) => input.txBytes);
          const merkleProofs = freshInputs.map((input) => input.merkleProof);
          const freshContinuityProof = freshResponse.continuityProof;

          const payload: BatchPayload = {
            heights,
            txBytesList,
            merkleProofs,
            continuityProof: freshContinuityProof,
          };
          const validationError = validateBatchPayload(
            payload,
            freshResponse.fromHeader,
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
            freshContinuityProof,
          );
        };

        // First attempt uses already-fetched data
        const submitWithCachedData = () => {
          const heights = batchInputs.map((input) => input.headerNumber);
          const txBytesList = batchInputs.map((input) => input.txBytes);
          const merkleProofs = batchInputs.map((input) => input.merkleProof);

          const payload: BatchPayload = {
            heights,
            txBytesList,
            merkleProofs,
            continuityProof: sharedContinuityProof,
          };
          const validationError = validateBatchPayload(
            payload,
            startHeight,
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

        // Try with cached data first, use fresh fetch for continuity retries
        let firstAttempt = true;
        const batchResult = await withContinuityRetry(
          () => {
            if (firstAttempt) {
              firstAttempt = false;
              return submitWithCachedData();
            }
            return submitFreshBatch();
          },
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
        onError?.(errorMsg);
        const fallback = await submitProofsIndividually(
          config,
          batchInputs.map((input) => input.txInfo),
          1000,
          onError,
        );
        successful += fallback.successful;
        failed += fallback.failed;
      }

      index += batchInputs.length;
    }
  }

  console.log(`📦 Batch complete: ${successful} successful, ${failed} failed`);

  return { successful, failed, batches };
}
