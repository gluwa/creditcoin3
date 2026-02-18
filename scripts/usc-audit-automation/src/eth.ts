/**
 * Ethereum RPC client for block queries
 */

import { JsonRpcProvider, WebSocketProvider } from "ethers";
import { withTimeout } from "./timeout.ts";

const RPC_TIMEOUT_MS = 30_000;

function getProvider(rpcUrl: string): JsonRpcProvider | WebSocketProvider {
  if (rpcUrl.startsWith("ws://") || rpcUrl.startsWith("wss://")) {
    return new WebSocketProvider(rpcUrl);
  }
  return new JsonRpcProvider(rpcUrl);
}

export async function getBlockNumber(rpcUrl: string): Promise<number> {
  const provider = getProvider(rpcUrl);
  try {
    const block = await withTimeout(provider.getBlockNumber(), RPC_TIMEOUT_MS);
    return Number(block);
  } finally {
    provider.destroy();
  }
}

export async function getBlockNumberByHash(
  rpcUrl: string,
  blockHash: string,
): Promise<number | null> {
  const provider = getProvider(rpcUrl);
  try {
    const block = await withTimeout(
      provider.getBlock(blockHash),
      RPC_TIMEOUT_MS,
    );
    const n = block?.number;
    return n != null ? Number(n) : null;
  } finally {
    provider.destroy();
  }
}

export async function checkRpcHealthy(rpcUrl: string): Promise<boolean> {
  const provider = getProvider(rpcUrl);
  try {
    await withTimeout(provider.getNetwork(), RPC_TIMEOUT_MS);
    return true;
  } catch {
    return false;
  } finally {
    provider.destroy();
  }
}
