/**
 * Unified exponential backoff utilities
 */

import {
  BASE_RECONNECT_DELAY_MS,
  MAX_RECONNECT_ATTEMPTS,
} from "../constants.ts";

/**
 * Calculate exponential backoff delay: baseDelay * 2^(attempt-1)
 */
export function getBackoffDelay(
  attempt: number,
  baseDelayMs: number,
): number {
  return baseDelayMs * Math.pow(2, attempt - 1);
}

/**
 * Calculate reconnection delay using default reconnect settings
 */
export function getReconnectDelay(attempt: number): number {
  return getBackoffDelay(attempt, BASE_RECONNECT_DELAY_MS);
}

export function logReconnectAttempt(
  name: string,
  attempt: number,
  delayMs: number,
): void {
  console.log(
    `⏳ Reconnecting to ${name} in ${delayMs}ms (attempt ${attempt}/${MAX_RECONNECT_ATTEMPTS})`,
  );
}

export function logReconnectFailed(name: string): void {
  console.error(`Max reconnection attempts exceeded for ${name}`);
}
