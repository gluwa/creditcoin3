/**
 * Block subscriber for source chain (Sepolia)
 *
 * Subscribes to new blocks via WebSocket and notifies when blocks are received.
 */

import { WebSocketProvider } from 'ethers';
import type { BlockInfo } from '../types.ts';
import { BaseSubscriber } from './baseSubscriber.ts';

export type BlockCallback = (block: BlockInfo) => void | Promise<void>;

export class BlockSubscriber extends BaseSubscriber {
  protected readonly name = 'source chain';
  private provider: WebSocketProvider | null = null;
  private onBlock: BlockCallback;
  private rpcUrl: string;

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

      this.provider.on('block', async (blockNumber: number) => {
        try {
          await this.handleBlock(blockNumber);
        } catch (error) {
          console.error(`Error handling block ${blockNumber}:`, error);
        }
      });

      this.provider.on('error', async (error: Error) => {
        console.error('Source chain provider error:', error);
        if (this.isRunning) await this.reconnect();
      });
    } catch (error) {
      console.error('Failed to connect to source chain:', error);
      if (this.isRunning) await this.reconnect();
    }
  }

  private async handleBlock(blockNumber: number): Promise<void> {
    if (!this.provider) return;

    try {
      const block = await this.provider.getBlock(blockNumber, true);
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
          if (typeof tx === 'string') {
            txHashes.push(tx);
          } else if (tx && typeof tx === 'object' && 'hash' in tx) {
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

  protected async cleanup(): Promise<void> {
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
