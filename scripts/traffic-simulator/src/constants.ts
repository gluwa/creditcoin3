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

// RPC settings (override via RPC_TIMEOUT_MS env var for slow/remote chains)
const _rpcTimeout = Number(Deno.env.get("RPC_TIMEOUT_MS"));
export const RPC_TIMEOUT_MS = _rpcTimeout > 0 ? _rpcTimeout : 30_000;
const _receiptTimeout = Number(Deno.env.get("RECEIPT_TIMEOUT_MS"));
export const RECEIPT_TIMEOUT_MS = _receiptTimeout > 0
  ? _receiptTimeout
  : 120_000;

// Simulation timeout — `eth_call` against `verifyAndEmit` with non-trivial
// calldata can take significantly longer than a normal RPC round-trip because
// the node has to run the full precompile (Merkle + continuity verification)
// inside the single-threaded RPC handler. Give it more headroom than the
// generic RPC timeout, but still bound it.
const _simTimeout = Number(Deno.env.get("SIMULATION_TIMEOUT_MS"));
export const SIMULATION_TIMEOUT_MS = _simTimeout > 0 ? _simTimeout : 90_000;

// Skip pre-flight `eth_call` simulation when the calldata exceeds this size,
// in bytes. Large `verifyAndEmit` payloads (batches, big tx witnesses) can
// take longer than any reasonable RPC timeout to simulate, even though the
// actual on-chain submission goes through fine. For those, skip simulation
// and rely on receipt-side error handling instead.
const _simSkip = Number(Deno.env.get("SIMULATION_SKIP_BYTES"));
export const SIMULATION_SKIP_BYTES = _simSkip > 0 ? _simSkip : 16_384;

// Transient network error retry settings (for socket hang up, ECONNRESET, etc.)
export const MAX_TRANSIENT_RETRIES = 3;
export const TRANSIENT_RETRY_BASE_DELAY_MS = 2_000;

// Gas limits
//
// These are now treated as **ceilings** rather than unconditional values.
// Submission code calls `eth_estimateGas` and pads the result; the
// constants below cap that estimate so a misbehaving node can't hand us
// an absurd value, and act as a fallback if estimation itself fails.
export const SINGLE_PROOF_GAS_LIMIT = 5_000_000n;
export const BATCH_PROOF_GAS_LIMIT = 10_000_000n;

// Floor on the dynamically-computed gasLimit. Anything below this is
// almost certainly a bad estimate (e.g. Frontier returning 21000 for a
// precompile call) and we want to reject it rather than ship a tx that
// will out-of-gas mid-call.
export const MIN_DYNAMIC_GAS_LIMIT = 200_000n;

// Buffer (`GAS_BUFFER_PERCENT`) applied on top of `eth_estimateGas`.
// Default 20%. Bigger proofs and Frontier RPC sometimes underestimate by
// more than EVM does, so we pad before capping.
const _gasBuffer = Number(Deno.env.get("GAS_BUFFER_PERCENT"));
export const GAS_BUFFER_PERCENT = Number.isFinite(_gasBuffer) && _gasBuffer >= 0
  ? BigInt(Math.floor(_gasBuffer))
  : 20n;

// Per-continuity-block extra gas (`GAS_PER_CONTINUITY_BLOCK`). Continuity
// verification cost scales with the number of roots in the continuity
// proof, and `estimateGas` doesn't always capture that growth precisely
// for large batches. Default 50k per block.
const _gasPerBlock = Number(Deno.env.get("GAS_PER_CONTINUITY_BLOCK"));
export const GAS_PER_CONTINUITY_BLOCK =
  Number.isFinite(_gasPerBlock) && _gasPerBlock >= 0
    ? BigInt(Math.floor(_gasPerBlock))
    : 50_000n;

// Fee settings
export const MIN_PRIORITY_FEE_GWEI = 1n;

// Precompile address
export const PRECOMPILE_ADDRESS = "0x0000000000000000000000000000000000000FD2";

// Function signatures
export const SINGLE_VERIFY_SIG =
  "verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))";
export const BATCH_VERIFY_SIG =
  "verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))";
