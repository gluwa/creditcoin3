/**
 * Dynamic gas-limit estimation for precompile transactions.
 *
 * Why dynamic instead of fixed? Shipping a fixed 5M / 10M `gasLimit` for
 * every submission has two failure modes:
 *
 *   1. Block-builder deprioritization. Validators ordering by effective gas
 *      price for the slot a tx takes up will deprioritize fat txs in favor
 *      of skinnier ones at the same fee, even when the fat tx pays more in
 *      absolute terms. The fat tx sits in the mempool and `tx.wait()`
 *      eventually times out.
 *   2. Upfront balance check. Nodes reject txs where
 *      `gasLimit * maxFeePerGas > sender.balance` even though the actual
 *      spend would have been a fraction of that. Easy to trip on a
 *      low-balance signer with `gasLimit = 10_000_000`.
 *
 * `eth_estimateGas` + a buffer is what production submitters do. The
 * existing `*_GAS_LIMIT` constants are kept as ceilings (and as fallback
 * values when estimation itself fails) rather than unconditional values.
 */

import { ethers, JsonRpcProvider } from "ethers";
import {
  GAS_BUFFER_PERCENT,
  GAS_PER_CONTINUITY_BLOCK,
  MIN_DYNAMIC_GAS_LIMIT,
  PRECOMPILE_ADDRESS,
  RPC_TIMEOUT_MS,
} from "../constants.ts";
import { withTimeout } from "../utils/retry.ts";
import { getErrorMessage } from "../utils/errors.ts";

interface ComputeGasLimitParams {
  provider: JsonRpcProvider;
  from: string;
  data: string;
  /**
   * Number of continuity-proof roots in this submission. `1` for single
   * proofs; `continuityProof.roots.length` for batches.
   */
  continuityBlocks: number;
  /**
   * Hard ceiling on the returned `gasLimit`. The dynamically-computed
   * value is `min(estimate * buffer + perBlockPadding, ceiling)`.
   */
  ceiling: bigint;
  /**
   * Fallback returned (along with a warning log) when `estimateGas`
   * itself errors. Typically the same value as `ceiling`.
   */
  fallback: bigint;
  /** Log label for diagnostics. */
  label: string;
}

/**
 * Compute a tight-but-safe `gasLimit` for a precompile submission.
 *
 * - Calls `eth_estimateGas` (under `RPC_TIMEOUT_MS`) — Frontier nodes can
 *   be slow to estimate because it runs the full precompile.
 * - Pads with `GAS_BUFFER_PERCENT` (default 20%).
 * - Adds `GAS_PER_CONTINUITY_BLOCK` (default 50k) per continuity root.
 * - Floors at `MIN_DYNAMIC_GAS_LIMIT` to reject obviously-bad estimates
 *   (e.g. 21000 from a misbehaving Frontier node).
 * - Caps at `ceiling` to keep a misbehaving estimator from returning
 *   absurd values.
 * - On `estimateGas` failure (revert mid-estimate, RPC timeout, etc.),
 *   logs a warning and returns `fallback` so submission can proceed
 *   instead of crashing the caller.
 *
 * The returned record includes the inputs we used so the caller can log
 * estimated vs. final gas at info level for diagnostics.
 */
export async function computeGasLimit(
  params: ComputeGasLimitParams,
): Promise<
  {
    gasLimit: bigint;
    estimated: bigint | null;
    paddedEstimate: bigint | null;
    cappedAtCeiling: boolean;
    usedFallback: boolean;
  }
> {
  const { provider, from, data, continuityBlocks, ceiling, fallback, label } =
    params;

  let estimated: bigint;
  try {
    estimated = await withTimeout(
      provider.estimateGas({ from, to: PRECOMPILE_ADDRESS, data }),
      RPC_TIMEOUT_MS,
      `${label} estimateGas`,
    );
  } catch (error) {
    console.warn(
      `⚠️  ${label}: estimateGas failed (${
        getErrorMessage(error)
      }), falling back to ceiling=${fallback}`,
    );
    return {
      gasLimit: fallback,
      estimated: null,
      paddedEstimate: null,
      cappedAtCeiling: false,
      usedFallback: true,
    };
  }

  // Apply percent buffer. `GAS_BUFFER_PERCENT = 0` is a no-op.
  let padded = estimated +
    (estimated * GAS_BUFFER_PERCENT) / 100n;

  // Per-continuity-block extra padding.
  if (continuityBlocks > 0 && GAS_PER_CONTINUITY_BLOCK > 0n) {
    padded += BigInt(continuityBlocks) * GAS_PER_CONTINUITY_BLOCK;
  }

  // Floor: anything below this is almost certainly a bad estimate.
  if (padded < MIN_DYNAMIC_GAS_LIMIT) {
    console.warn(
      `⚠️  ${label}: padded estimate ${padded} below floor ${MIN_DYNAMIC_GAS_LIMIT} (raw=${estimated}); using floor`,
    );
    padded = MIN_DYNAMIC_GAS_LIMIT;
  }

  // Cap at ceiling.
  let capped = false;
  let gasLimit = padded;
  if (gasLimit > ceiling) {
    capped = true;
    gasLimit = ceiling;
  }

  return {
    gasLimit,
    estimated,
    paddedEstimate: padded,
    cappedAtCeiling: capped,
    usedFallback: false,
  };
}

/**
 * Pre-flight balance check. Many nodes reject a tx outright when
 * `gasLimit * maxFeePerGas > sender.balance` even when the actual spend
 * would be a small fraction of that — and on some Frontier-derived nodes
 * the rejection manifests as silent non-propagation rather than a clean
 * error. Surface a clear log line before broadcast so operators can see
 * what's happening.
 *
 * Returns `null` if everything looks fine, or an explanatory string if
 * the balance is tight; the caller decides whether to log/throw.
 */
export async function checkBalanceForGas(
  provider: JsonRpcProvider,
  from: string,
  gasLimit: bigint,
  feeOverrides: ethers.TransactionRequest,
): Promise<string | null> {
  // Use the higher of maxFeePerGas / gasPrice for the worst-case calc.
  const feeCap = (feeOverrides.maxFeePerGas as bigint | undefined) ??
    (feeOverrides.gasPrice as bigint | undefined);
  if (feeCap === undefined) return null;

  const required = gasLimit * feeCap;
  const balance = await withTimeout(
    provider.getBalance(from),
    RPC_TIMEOUT_MS,
    "Balance check",
  );

  if (balance >= required) return null;

  return `signer balance ${balance} < required upfront ${required} ` +
    `(gasLimit=${gasLimit} * feeCap=${feeCap})`;
}
