/**
 * HTTP request executor with timing.
 * Fires GET requests and captures status, latency, and error details.
 */

import type { RequestResult, StressRequest } from "./types.ts";

const DEFAULT_TIMEOUT_MS = 30_000;

/**
 * Execute a single HTTP GET request and measure its performance.
 */
export async function executeRequest(
  request: StressRequest,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<RequestResult> {
  const start = performance.now();
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetch(request.url, {
      method: "GET",
      signal: controller.signal,
      headers: { "Accept": "application/json" },
    });

    const latencyMs = performance.now() - start;

    let errorCode: string | undefined;
    let errorMessage: string | undefined;

    if (!response.ok) {
      try {
        const body = await response.json();
        errorCode = body.code;
        errorMessage = body.message;
      } catch {
        // Body wasn't JSON; that's fine
      }
    } else {
      // Consume the response body to avoid leaking resources
      await response.arrayBuffer();
    }

    return {
      status: response.status,
      latencyMs,
      kind: request.kind,
      errorCode,
      errorMessage,
    };
  } catch (error) {
    const latencyMs = performance.now() - start;

    // Distinguish client-side timeouts from actual network errors
    if (error instanceof DOMException && error.name === "AbortError") {
      return {
        status: 0,
        latencyMs,
        kind: request.kind,
        errorCode: "Timeout",
        errorMessage: `Request timed out after ${timeoutMs}ms`,
      };
    }

    let errorMessage = "Unknown error";
    if (error instanceof Error) {
      errorMessage = error.message;
    }

    return {
      status: 0,
      latencyMs,
      kind: request.kind,
      errorCode: "NetworkError",
      errorMessage,
    };
  } finally {
    clearTimeout(timeoutId);
  }
}
