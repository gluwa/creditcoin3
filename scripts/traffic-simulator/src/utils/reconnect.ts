/**
 * Shared reconnection utility for subscribers
 */

import {
  BASE_RECONNECT_DELAY_MS,
  MAX_RECONNECT_ATTEMPTS,
} from "../constants.ts";

/**
 * Calculate delay for next reconnection attempt with exponential backoff
 */
export function getReconnectDelay(attempt: number): number {
  return BASE_RECONNECT_DELAY_MS * Math.pow(2, attempt - 1);
}

/**
 * Log reconnection attempt
 */
export function logReconnectAttempt(
  name: string,
  attempt: number,
  delayMs: number,
): void {
  console.log(
    `⏳ Reconnecting to ${name} in ${delayMs}ms (attempt ${attempt}/${MAX_RECONNECT_ATTEMPTS})`,
  );
}

/**
 * Log max reconnection attempts exceeded
 */
export function logReconnectFailed(name: string): void {
  console.error(`Max reconnection attempts exceeded for ${name}`);
}

/**
 * Sleep for a given duration
 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
