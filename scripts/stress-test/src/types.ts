/**
 * Type definitions for the stress test tool
 */

/** Stress test mode */
export type StressMode = "valid" | "invalid" | "mixed";

/** A pre-generated request to fire at the API */
export interface StressRequest {
  /** Full URL to GET */
  url: string;
  /** Whether this is expected to be a valid or invalid request */
  kind: "valid" | "invalid";
  /** Description of the invalid category (if invalid) */
  invalidCategory?: string;
}

/** Result of a single HTTP request */
export interface RequestResult {
  /** HTTP status code (0 if network error) */
  status: number;
  /** Response time in milliseconds */
  latencyMs: number;
  /** Whether request was a valid or invalid one */
  kind: "valid" | "invalid";
  /** Error code from API response body, if any */
  errorCode?: string;
  /** Error message, if any */
  errorMessage?: string;
}

/** Block data fetched from source chain for valid request generation */
export interface BlockData {
  blockNumber: number;
  txCount: number;
  txHashes: string[];
}

/** Stress test configuration */
export interface StressConfig {
  /** Test mode */
  mode: StressMode;
  /** Proof-gen API base URL */
  apiUrl: string;
  /** Chain key */
  chainKey: number;
  /** Source chain HTTP RPC URL (required for valid/mixed) */
  sourceRpcUrl?: string;
  /** Target requests per second */
  rps: number;
  /** Max concurrent requests */
  concurrency: number;
  /** Test duration in seconds */
  duration: number;
  /** Ratio of valid requests in mixed mode (0.0-1.0) */
  mixRatio: number;
  /** Optional block range [start, end] for valid requests */
  blockRange?: [number, number];
  /** Request timeout in milliseconds */
  timeout: number;
  /** Enable verbose logging of individual requests */
  verbose: boolean;
}
