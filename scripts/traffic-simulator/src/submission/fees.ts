/**
 * Fee estimation and bumping for precompile transactions.
 */

import { ethers, JsonRpcProvider } from "ethers";
import {
  FEE_HEADROOM_PERCENT,
  MAX_PRIORITY_FEE_GWEI,
  MIN_PRIORITY_FEE_GWEI,
  RPC_TIMEOUT_MS,
} from "../constants.ts";
import { withTimeout } from "../utils/retry.ts";

const GWEI = 1_000_000_000n;

/**
 * Apply `FEE_HEADROOM_PERCENT` extra on top of `value`.
 * `headroom = 0` is a no-op; default 100 means 2×.
 */
function applyHeadroom(value: bigint): bigint {
  if (FEE_HEADROOM_PERCENT === 0n) return value;
  return value + (value * FEE_HEADROOM_PERCENT) / 100n;
}

/**
 * Clamp the priority fee to `[MIN_PRIORITY_FEE_GWEI, MAX_PRIORITY_FEE_GWEI]`.
 * `MAX_PRIORITY_FEE_GWEI = 0` disables the upper bound.
 */
function clampPriority(priority: bigint): bigint {
  const minPriority = MIN_PRIORITY_FEE_GWEI * GWEI;
  let p = priority < minPriority ? minPriority : priority;
  if (MAX_PRIORITY_FEE_GWEI > 0n) {
    const maxPriority = MAX_PRIORITY_FEE_GWEI * GWEI;
    if (p > maxPriority) p = maxPriority;
  }
  return p;
}

export async function getFeeOverrides(
  provider: JsonRpcProvider,
): Promise<ethers.TransactionRequest> {
  const feeData = await withTimeout(
    provider.getFeeData(),
    RPC_TIMEOUT_MS,
    "Fee data",
  );

  if (feeData.maxFeePerGas !== null) {
    // EIP-1559 path: bump both the cap *and* the tip with headroom so a
    // single base-fee jump doesn't strand the tx, then enforce the
    // configurable tip floor and (optional) ceiling. `maxFeePerGas` must
    // remain >= tip after bumping or ethers will reject the tx.
    const suggestedTip = feeData.maxPriorityFeePerGas ??
      MIN_PRIORITY_FEE_GWEI * GWEI;
    const priority = clampPriority(applyHeadroom(suggestedTip));
    let maxFee = applyHeadroom(feeData.maxFeePerGas);
    if (maxFee < priority) maxFee = priority;
    return {
      maxFeePerGas: maxFee,
      maxPriorityFeePerGas: priority,
    };
  }

  // Legacy path: no EIP-1559 fee data, just bump the suggested gasPrice.
  const suggested = feeData.gasPrice ?? MIN_PRIORITY_FEE_GWEI * GWEI;
  return { gasPrice: applyHeadroom(suggested) };
}

export function bumpFees(
  overrides: ethers.TransactionRequest,
): ethers.TransactionRequest {
  const bump = (
    v: bigint | null | undefined,
  ) => (v ? (v * 120n) / 100n + 1n : undefined);
  return {
    maxFeePerGas: bump(overrides.maxFeePerGas as bigint),
    maxPriorityFeePerGas: bump(overrides.maxPriorityFeePerGas as bigint),
    gasPrice: bump(overrides.gasPrice as bigint),
  };
}
