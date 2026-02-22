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

/** Safely destroy a provider, ensuring WebSocket connections are fully closed. */
function destroyProvider(
  provider: JsonRpcProvider | WebSocketProvider,
): void {
  try {
    if (provider instanceof WebSocketProvider) {
      const ws = provider.websocket;
      provider.destroy();
      // WebSocketProvider.destroy() may not fully close the underlying socket
      if (ws && typeof (ws as { close?: () => void }).close === "function") {
        (ws as { close: () => void }).close();
      }
    } else {
      provider.destroy();
    }
  } catch {
    // Best-effort cleanup
  }
}

export async function getBlockNumber(rpcUrl: string): Promise<number> {
  const provider = getProvider(rpcUrl);
  try {
    const block = await withTimeout(provider.getBlockNumber(), RPC_TIMEOUT_MS);
    return Number(block);
  } finally {
    destroyProvider(provider);
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
    destroyProvider(provider);
  }
}

export async function checkRpcHealthy(rpcUrl: string): Promise<boolean> {
  const provider = getProvider(rpcUrl);
  try {
    // Verify connectivity and that the node returns a reasonable block number
    const [, blockNumber] = await withTimeout(
      Promise.all([provider.getNetwork(), provider.getBlockNumber()]),
      RPC_TIMEOUT_MS,
    );
    // A block number of 0 likely means the node is not synced
    return blockNumber > 0;
  } catch {
    return false;
  } finally {
    destroyProvider(provider);
  }
}
