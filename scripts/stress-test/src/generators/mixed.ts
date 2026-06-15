/**
 * Generates a mix of valid and invalid requests at a configurable ratio.
 */

import type { BlockData, StressRequest } from "../types.ts";
import { generateValidRequests } from "./valid.ts";
import { generateInvalidRequests } from "./invalid.ts";

/**
 * Generate mixed valid + invalid requests.
 *
 * @param validRatio - Fraction of requests that should be valid (0.0-1.0)
 */
export function generateMixedRequests(
  apiUrl: string,
  chainKey: number,
  blocks: BlockData[],
  count: number,
  validRatio: number,
): StressRequest[] {
  const validCount = Math.round(count * validRatio);
  const invalidCount = count - validCount;

  const valid = generateValidRequests(apiUrl, chainKey, blocks, validCount);
  const invalid = generateInvalidRequests(apiUrl, chainKey, invalidCount);

  // Interleave randomly
  const mixed = [...valid, ...invalid];
  for (let i = mixed.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [mixed[i], mixed[j]] = [mixed[j], mixed[i]];
  }

  return mixed;
}
