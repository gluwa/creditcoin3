/**
 * Fetch with timeout to avoid hanging on unresponsive servers.
 */

const DEFAULT_TIMEOUT_MS = 30_000;

export async function fetchWithTimeout(
  url: string,
  init: RequestInit & { timeoutMs?: number } = {},
): Promise<Response> {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...fetchInit } = init;
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, {
      ...fetchInit,
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timeoutId);
  }
}
