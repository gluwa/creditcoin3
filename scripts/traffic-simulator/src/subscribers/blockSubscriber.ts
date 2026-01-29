/**
 * Block subscriber for source chain (Sepolia)
 *
 * Subscribes to new blocks via WebSocket and notifies when blocks are received.
 */

import { WebSocketProvider } from "ethers";
import type { BlockInfo } from "../types.ts";
import { BaseSubscriber } from "./baseSubscriber.ts";
import { withTimeout } from "../utils/retry.ts";

export type BlockCallback = (block: BlockInfo) => void | Promise<void>;

// Timeout for getBlock RPC call (30 seconds)
const GET_BLOCK_TIMEOUT_MS = 30_000;

// If no blocks received for this duration, assume connection is stale and reconnect
// Sepolia produces a block every ~12 seconds, so 90 seconds should be safe
const BLOCK_WATCHDOG_TIMEOUT_MS = 90_000;

export class BlockSubscriber extends BaseSubscriber {
  protected readonly name = "source chain";
  private provider: WebSocketProvider | null = null;
  private onBlock: BlockCallback;
  private rpcUrl: string;
  private lastBlockTime: number = 0;
  private watchdogTimer: ReturnType<typeof setInterval> | null = null;

  constructor(rpcUrl: string, onBlock: BlockCallback) {
    super();
    this.rpcUrl = rpcUrl;
    this.onBlock = onBlock;
  }

  protected async connect(): Promise<void> {
    try {
      console.log(`🔗 Connecting to source chain: ${this.rpcUrl}`);
      this.provider = new WebSocketProvider(this.rpcUrl);
      await this.provider.ready;

      const network = await this.provider.getNetwork();
      console.log(`✅ Connected to source chain (chainId: ${network.chainId})`);
      this.resetReconnectAttempts();
      this.lastBlockTime = Date.now();

      this.provider.on("block", async (blockNumber: number) => {
        this.lastBlockTime = Date.now();
        try {
          await this.handleBlock(blockNumber);
        } catch (error) {
          console.error(`Error handling block ${blockNumber}:`, error);
        }
      });

      this.provider.on("error", async (error: Error) => {
        console.error("Source chain provider error:", error);
        if (this.isRunning) await this.reconnect();
      });

      // Start watchdog timer to detect stale connections
      // This handles cases where the WebSocket silently disconnects without firing "error"
      this.startWatchdog();
    } catch (error) {
      console.error("Failed to connect to source chain:", error);
      if (this.isRunning) await this.reconnect();
    }
  }

  private startWatchdog(): void {
    this.stopWatchdog();
    this.watchdogTimer = setInterval(async () => {
      if (!this.isRunning) return;

      const timeSinceLastBlock = Date.now() - this.lastBlockTime;
      if (timeSinceLastBlock > BLOCK_WATCHDOG_TIMEOUT_MS) {
        console.warn(
          `⚠️  No blocks received for ${
            Math.round(timeSinceLastBlock / 1000)
          }s, reconnecting...`,
        );
        await this.reconnect();
      }
    }, 30_000); // Check every 30 seconds
  }

  private stopWatchdog(): void {
    if (this.watchdogTimer) {
      clearInterval(this.watchdogTimer);
      this.watchdogTimer = null;
    }
  }

  private async handleBlock(blockNumber: number): Promise<void> {
    if (!this.provider) return;

    try {
      const block = await this.getBlockWithTimeout(blockNumber);
      if (!block) {
        console.warn(`Block ${blockNumber} not found`);
        return;
      }

      const txHashes: string[] = [];
      if (block.prefetchedTransactions?.length > 0) {
        for (const tx of block.prefetchedTransactions) {
          txHashes.push(tx.hash);
        }
      } else if (Array.isArray(block.transactions)) {
        for (const tx of block.transactions) {
          if (typeof tx === "string") {
            txHashes.push(tx);
          } else if (tx && typeof tx === "object" && "hash" in tx) {
            const hash = (tx as { hash?: string }).hash;
            if (hash) txHashes.push(hash);
          }
        }
      }

      await this.onBlock({ blockNumber, txHashes, timestamp: Date.now() });
    } catch (error) {
      console.error(`Error fetching block ${blockNumber}:`, error);
    }
  }

  private getBlockWithTimeout(
    blockNumber: number,
  ): Promise<Awaited<ReturnType<WebSocketProvider["getBlock"]>>> {
    if (!this.provider) return Promise.resolve(null);

    return withTimeout(
      this.provider.getBlock(blockNumber, true),
      GET_BLOCK_TIMEOUT_MS,
      `getBlock(${blockNumber})`,
    );
  }

  protected async cleanup(): Promise<void> {
    this.stopWatchdog();
    if (this.provider) {
      try {
        this.provider.removeAllListeners();
        await this.provider.destroy();
      } catch { /* ignore */ }
      this.provider = null;
    }
  }

  get isConnected(): boolean {
    return this.provider !== null && this.isRunning;
  }
}
