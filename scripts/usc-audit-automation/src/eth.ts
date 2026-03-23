/**
 * Ethereum RPC client for block queries
 */

import { JsonRpcProvider, WebSocketProvider } from "ethers";
import { withTimeout } from "./timeout.ts";

const RPC_TIMEOUT_MS = 30_000;

/** Retries help with transient public-RPC / CI flakes (rate limits, TLS, cold WS). */
const RPC_HEALTH_MAX_ATTEMPTS = 3;
const RPC_HEALTH_RETRY_BASE_MS = 1_000;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

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

async function checkRpcHealthyOnce(rpcUrl: string): Promise<boolean> {
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

/** Returns true if any attempt succeeds. */
export async function checkRpcHealthy(rpcUrl: string): Promise<boolean> {
  for (let attempt = 1; attempt <= RPC_HEALTH_MAX_ATTEMPTS; attempt++) {
    const ok = await checkRpcHealthyOnce(rpcUrl);
    if (ok) return true;
    if (attempt < RPC_HEALTH_MAX_ATTEMPTS) {
      await sleep(RPC_HEALTH_RETRY_BASE_MS * attempt);
    }
  }
  return false;
}
