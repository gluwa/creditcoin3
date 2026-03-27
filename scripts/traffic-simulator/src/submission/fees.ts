/**
 * Fee estimation and bumping for precompile transactions.
 */

import { ethers, JsonRpcProvider } from "ethers";
import { MIN_PRIORITY_FEE_GWEI, RPC_TIMEOUT_MS } from "../constants.ts";
import { withTimeout } from "../utils/retry.ts";

export async function getFeeOverrides(
  provider: JsonRpcProvider,
): Promise<ethers.TransactionRequest> {
  const feeData = await withTimeout(
    provider.getFeeData(),
    RPC_TIMEOUT_MS,
    "Fee data",
  );
  const minPriority = MIN_PRIORITY_FEE_GWEI * 1_000_000_000n;

  if (feeData.maxFeePerGas !== null) {
    const priority = feeData.maxPriorityFeePerGas ?? minPriority;
    return {
      maxFeePerGas: feeData.maxFeePerGas,
      maxPriorityFeePerGas: priority < minPriority ? minPriority : priority,
    };
  }

  return { gasPrice: feeData.gasPrice ?? minPriority };
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
