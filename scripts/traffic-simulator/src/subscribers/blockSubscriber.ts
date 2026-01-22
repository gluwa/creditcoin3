/**
 * Block subscriber for source chain (Sepolia)
 *
 * Subscribes to new blocks via WebSocket and notifies when blocks are received.
 */

import { WebSocketProvider } from 'ethers';
import type { BlockInfo } from '../types.ts';

/**
 * Callback type for block notifications
 */
export type BlockCallback = (block: BlockInfo) => void | Promise<void>;

/**
 * Subscribes to new blocks on the source chain
 */
export class BlockSubscriber {
  private provider: WebSocketProvider | null = null;
  private onBlock: BlockCallback;
  private rpcUrl: string;
  private isRunning = false;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 10;
  private reconnectDelayMs = 5000;

  constructor(rpcUrl: string, onBlock: BlockCallback) {
    this.rpcUrl = rpcUrl;
    this.onBlock = onBlock;
  }

  /**
   * Start subscribing to blocks
   */
  async start(): Promise<void> {
    if (this.isRunning) {
      return;
    }

    this.isRunning = true;
    await this.connect();
  }

  /**
   * Connect to the WebSocket provider
   */
  private async connect(): Promise<void> {
    try {
      console.log(`🔗 Connecting to source chain: ${this.rpcUrl}`);
      this.provider = new WebSocketProvider(this.rpcUrl);

      // Wait for provider to be ready
      await this.provider.ready;

      const network = await this.provider.getNetwork();
      console.log(`✅ Connected to source chain (chainId: ${network.chainId})`);

      // Reset reconnect attempts on successful connection
      this.reconnectAttempts = 0;

      // Subscribe to new blocks
      this.provider.on('block', async (blockNumber: number) => {
        try {
          await this.handleBlock(blockNumber);
        } catch (error) {
          console.error(`Error handling block ${blockNumber}:`, error);
        }
      });

      // Handle provider errors
      this.provider.on('error', async (error: Error) => {
        console.error('Source chain provider error:', error);
        if (this.isRunning) {
          await this.reconnect();
        }
      });
    } catch (error) {
      console.error('Failed to connect to source chain:', error);
      if (this.isRunning) {
        await this.reconnect();
      }
    }
  }

  /**
   * Handle a new block
   */
  private async handleBlock(blockNumber: number): Promise<void> {
    if (!this.provider) {
      return;
    }

    try {
      // Get block with transactions
      const block = await this.provider.getBlock(blockNumber, true);

      if (!block) {
        console.warn(`Block ${blockNumber} not found`);
        return;
      }

      // Extract transaction hashes in block order
      const txHashes: string[] = [];
      if (block.prefetchedTransactions && block.prefetchedTransactions.length > 0) {
        for (const tx of block.prefetchedTransactions) {
          txHashes.push(tx.hash);
        }
      } else if (Array.isArray(block.transactions) && block.transactions.length > 0) {
        for (const tx of block.transactions) {
          if (typeof tx === 'string') {
            txHashes.push(tx);
          } else if (tx && typeof tx === 'object' && 'hash' in tx) {
            const hash = (tx as { hash?: string }).hash;
            if (hash) {
              txHashes.push(hash);
            }
          }
        }
      }

      const blockInfo: BlockInfo = {
        blockNumber,
        txHashes,
        timestamp: Date.now(),
      };

      // Notify callback
      await this.onBlock(blockInfo);
    } catch (error) {
      console.error(`Error fetching block ${blockNumber}:`, error);
    }
  }

  /**
   * Attempt to reconnect
   */
  private async reconnect(): Promise<void> {
    if (!this.isRunning) {
      return;
    }

    this.reconnectAttempts++;

    if (this.reconnectAttempts > this.maxReconnectAttempts) {
      console.error('Max reconnection attempts exceeded for source chain');
      this.isRunning = false;
      return;
    }

    const delay = this.reconnectDelayMs * Math.pow(2, this.reconnectAttempts - 1);
    console.log(
      `⏳ Reconnecting to source chain in ${delay}ms (attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts})`,
    );

    // Clean up old provider
    await this.cleanup();

    // Wait before reconnecting
    await new Promise((resolve) => setTimeout(resolve, delay));

    if (this.isRunning) {
      await this.connect();
    }
  }

  /**
   * Clean up provider resources
   */
  private async cleanup(): Promise<void> {
    if (this.provider) {
      try {
        this.provider.removeAllListeners();
        await this.provider.destroy();
      } catch {
        // Ignore cleanup errors
      }
      this.provider = null;
    }
  }

  /**
   * Stop subscribing to blocks
   */
  async stop(): Promise<void> {
    console.log('⏹️  Stopping source chain subscriber...');
    this.isRunning = false;
    await this.cleanup();
    console.log('✅ Source chain subscriber stopped');
  }

  /**
   * Check if connected
   */
  get isConnected(): boolean {
    return this.provider !== null && this.isRunning;
  }
}
