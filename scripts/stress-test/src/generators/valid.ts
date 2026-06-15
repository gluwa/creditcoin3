/**
 * Generates valid API requests from real block data.
 *
 * Valid requests target:
 * - GET /api/v1/proof/{chain_key}/{block_number}/{tx_index}
 * - GET /api/v1/proof-by-tx/{chain_key}/{tx_hash}
 */

import type { BlockData, StressRequest } from "../types.ts";

/**
 * Generate valid request URLs from fetched block data.
 *
 * First generates one unique request per block+tx pair (maximizing unique
 * proofs to avoid hitting the API cache). Once unique combos are exhausted,
 * cycles back through them. Alternates between proof-by-index and
 * proof-by-tx endpoints.
 */
export function generateValidRequests(
  apiUrl: string,
  chainKey: number,
  blocks: BlockData[],
  count: number,
): StressRequest[] {
  if (blocks.length === 0) {
    throw new Error("No blocks available for generating valid requests");
  }

  // Build a pool of all unique block+tx combinations first
  const uniqueRequests: StressRequest[] = [];
  for (const block of blocks) {
    if (block.txCount === 0) continue;
    for (let txIdx = 0; txIdx < block.txCount; txIdx++) {
      // proof-by-index
      uniqueRequests.push({
        url: `${apiUrl}/api/v1/proof/${chainKey}/${block.blockNumber}/${txIdx}`,
        kind: "valid",
      });
      // proof-by-tx for the same transaction
      if (txIdx < block.txHashes.length) {
        uniqueRequests.push({
          url: `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${
            block.txHashes[txIdx]
          }`,
          kind: "valid",
        });
      }
    }
  }

  if (uniqueRequests.length === 0) {
    throw new Error(
      "No blocks with transactions found for generating valid requests",
    );
  }

  console.log(
    `  Unique block+tx request combinations: ${uniqueRequests.length}`,
  );
  if (count > uniqueRequests.length) {
    console.log(
      `  NOTE: Pool size (${count}) exceeds unique combos (${uniqueRequests.length}), requests will repeat and hit API cache`,
    );
  }

  // Fill the pool, cycling through unique requests
  const requests: StressRequest[] = [];
  for (let i = 0; i < count; i++) {
    requests.push(uniqueRequests[i % uniqueRequests.length]);
  }

  return requests;
}
