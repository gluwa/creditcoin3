/**
 * Shared constants for the traffic simulator
 */

// Reconnection settings
export const MAX_RECONNECT_ATTEMPTS = 10;
export const BASE_RECONNECT_DELAY_MS = 5_000;

// Continuity mismatch retry settings
export const MAX_CONTINUITY_RETRIES = 2;
export const CONTINUITY_RETRY_DELAY_MS = 15_000;

// Submission delays
export const SINGLE_SUBMISSION_DELAY_MS = 500;

// Proof API settings
export const PROOF_API_TIMEOUT_MS = 30_000;
export const PROOF_API_MAX_RETRIES = 5;
export const PROOF_API_BASE_DELAY_MS = 2_000;

// RPC settings
export const RPC_TIMEOUT_MS = 30_000;
export const RECEIPT_TIMEOUT_MS = 120_000;

// Transient network error retry settings (for socket hang up, ECONNRESET, etc.)
export const MAX_TRANSIENT_RETRIES = 3;
export const TRANSIENT_RETRY_BASE_DELAY_MS = 2_000;

// Gas limits
export const SINGLE_PROOF_GAS_LIMIT = 5_000_000n;
export const BATCH_PROOF_GAS_LIMIT = 10_000_000n;

// Fee settings
export const MIN_PRIORITY_FEE_GWEI = 1n;

// Precompile address
export const PRECOMPILE_ADDRESS = "0x0000000000000000000000000000000000000FD2";

// Function signatures
export const SINGLE_VERIFY_SIG =
  "verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))";
export const BATCH_VERIFY_SIG =
  "verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))";
