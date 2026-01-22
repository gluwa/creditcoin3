/**
 * Shared retry utilities
 */

import { CONTINUITY_RETRY_DELAY_MS, MAX_CONTINUITY_RETRIES } from '../constants.ts';
import { isContinuityMismatchError } from '../submitter/proofUtils.ts';
import { sleep } from './reconnect.ts';

/**
 * Execute a function with continuity mismatch retry logic
 */
export async function withContinuityRetry<T>(
  fn: () => Promise<T>,
  label: string,
): Promise<T> {
  for (let attempt = 0; attempt <= MAX_CONTINUITY_RETRIES; attempt++) {
    try {
      return await fn();
    } catch (error) {
      if (isContinuityMismatchError(error) && attempt < MAX_CONTINUITY_RETRIES) {
        console.warn(
          `⚠️  Continuity mismatch for ${label}, retrying in ${
            CONTINUITY_RETRY_DELAY_MS / 1000
          }s...`,
        );
        await sleep(CONTINUITY_RETRY_DELAY_MS);
        continue;
      }
      throw error;
    }
  }
  // This should never be reached, but TypeScript needs it
  throw new Error(`Retry failed for ${label}`);
}
