/**
 * Shared retry utilities
 */

import {
  CONTINUITY_RETRY_DELAY_MS,
  MAX_CONTINUITY_RETRIES,
} from "../constants.ts";
import { isContinuityMismatchError } from "./errors.ts";
import { sleep } from "./sleep.ts";

/**
 * Wrap a promise with a timeout. Rejects with an error if the timeout is reached.
 *
 * Implementation notes:
 * - When the timeout wins the race, the inner `promise` is still pending. If it
 *   later rejects (e.g. ethers' own request timeout firing on the abandoned
 *   call), that rejection has no listener and surfaces as an unhandled
 *   promise rejection, which crashes Deno. Attaching a no-op `.catch` on the
 *   inner promise turns that late rejection into a silent drop.
 * - Clear the timeout when the inner promise settles first so we don't keep
 *   dangling timers alive on the event loop.
 */
export function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  label: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${label} timed out after ${ms}ms`)),
      ms,
    );
  });
  // Swallow any late rejection from the abandoned inner promise after the
  // timeout has already won the race. Without this, ethers' own internal
  // request timeout firing ~30s later produces an unhandled rejection and
  // takes down the whole process.
  promise.catch(() => {});
  return Promise.race([promise, timeout]).finally(() => {
    if (timer !== undefined) clearTimeout(timer);
  });
}

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
      if (
        isContinuityMismatchError(error) && attempt < MAX_CONTINUITY_RETRIES
      ) {
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
