/**
 * Ethereum RPC client for block queries
 */

import { JsonRpcProvider, WebSocketProvider } from "ethers";

function getProvider(rpcUrl: string): JsonRpcProvider | WebSocketProvider {
  if (rpcUrl.startsWith("ws://") || rpcUrl.startsWith("wss://")) {
    return new WebSocketProvider(rpcUrl);
  }
  return new JsonRpcProvider(rpcUrl);
}

export async function getBlockNumber(rpcUrl: string): Promise<number> {
  const provider = getProvider(rpcUrl);
  try {
    const block = await provider.getBlockNumber();
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
    const block = await provider.getBlock(blockHash);
    const n = block?.number;
    return n != null ? Number(n) : null;
  } finally {
    provider.destroy();
  }
}

export async function checkRpcHealthy(rpcUrl: string): Promise<boolean> {
  try {
    const provider = getProvider(rpcUrl);
    await provider.getNetwork();
    provider.destroy();
    return true;
  } catch {
    return false;
  }
}
