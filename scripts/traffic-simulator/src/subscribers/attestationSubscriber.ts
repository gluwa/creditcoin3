/**
 * Attestation subscriber for Creditcoin3
 *
 * Subscribes to BlockAttested and CheckpointReached events via WebSocket.
 */

import { ApiPromise, WsProvider } from '@polkadot/api';

/**
 * Callback type for attestation notifications
 */
export type AttestationCallback = (blockNumber: number) => void | Promise<void>;

/**
 * Subscribes to attestation events on Creditcoin3
 */
export class AttestationSubscriber {
  private api: ApiPromise | null = null;
  private provider: WsProvider | null = null;
  private unsubscribe: (() => void) | null = null;
  private wsUrl: string;
  private chainKey: number;
  private onAttested: AttestationCallback;
  private isRunning = false;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 10;
  private reconnectDelayMs = 5000;

  constructor(wsUrl: string, chainKey: number, onAttested: AttestationCallback) {
    this.wsUrl = wsUrl;
    this.chainKey = chainKey;
    this.onAttested = onAttested;
  }

  /**
   * Start subscribing to attestation events
   */
  async start(): Promise<void> {
    if (this.isRunning) {
      return;
    }

    this.isRunning = true;
    await this.connect();
  }

  /**
   * Connect to Creditcoin3
   */
  private async connect(): Promise<void> {
    try {
      console.log(`🔗 Connecting to Creditcoin3: ${this.wsUrl}`);

      this.provider = new WsProvider(this.wsUrl);

      // Handle WebSocket errors
      this.provider.on('error', async () => {
        console.error('CC3 WebSocket error');
        if (this.isRunning) {
          await this.reconnect();
        }
      });

      this.provider.on('disconnected', async () => {
        console.warn('CC3 WebSocket disconnected');
        if (this.isRunning) {
          await this.reconnect();
        }
      });

      this.api = await ApiPromise.create({ 
        provider: this.provider,
        noInitWarn: true,  // Suppress "RPC methods not decorated" warnings
      });
      await this.api.isReady;

      const chain = await this.api.rpc.system.chain();
      console.log(`✅ Connected to Creditcoin3 (${chain})`);

      // Reset reconnect attempts on successful connection
      this.reconnectAttempts = 0;

      // Subscribe to system events
      // deno-lint-ignore no-explicit-any
      this.unsubscribe = (await this.api.query.system.events((events: any) => {
        this.handleEvents(events);
      })) as unknown as () => void;
    } catch (error) {
      console.error('Failed to connect to Creditcoin3:', error);
      if (this.isRunning) {
        await this.reconnect();
      }
    }
  }

  /**
   * Handle system events
   */
  private handleEvents(events: unknown): void {
    // Type assertion for polkadot events
    const eventRecords = events as Array<{
      event: {
        section: string;
        method: string;
        data: unknown[];
      };
    }>;

    for (const record of eventRecords) {
      const { event } = record;

      // Check for attestation events
      if (event.section === 'attestation') {
        if (event.method === 'BlockAttested') {
          this.handleBlockAttested(event.data);
        } else if (event.method === 'CheckpointReached') {
          this.handleCheckpointReached(event.data);
        }
      }
    }
  }

  /**
   * Handle BlockAttested event
   */
  private handleBlockAttested(data: unknown[]): void {
    try {
      // BlockAttested(chain_key, header_number, digest)
      const eventChainKey = this.toNumber(data[0]);
      const headerNumber = this.toNumber(data[1]);

      if (eventChainKey === this.chainKey) {
        console.log(`📢 BlockAttested: block ${headerNumber} on chain ${eventChainKey}`);
        this.onAttested(headerNumber);
      }
    } catch (error) {
      console.error('Error parsing BlockAttested event:', error);
    }
  }

  /**
   * Handle CheckpointReached event
   */
  private handleCheckpointReached(data: unknown[]): void {
    try {
      // CheckpointReached(chain_key, checkpoint)
      const eventChainKey = this.toNumber(data[0]);
      const checkpoint = data[1] as { block_number?: number; blockNumber?: number };
      const blockNumber = checkpoint.block_number ?? checkpoint.blockNumber;

      if (eventChainKey === this.chainKey && blockNumber !== undefined) {
        // Filter out corrupted/invalid block numbers
        if (blockNumber > 10_000_000_000) {
          console.warn(`Ignoring invalid checkpoint block number: ${blockNumber}`);
          return;
        }

        console.log(`📢 CheckpointReached: block ${blockNumber} on chain ${eventChainKey}`);
        this.onAttested(blockNumber);
      }
    } catch (error) {
      console.error('Error parsing CheckpointReached event:', error);
    }
  }

  /**
   * Convert polkadot type to number
   */
  private toNumber(value: unknown): number {
    if (typeof value === 'number') {
      return value;
    }
    if (typeof value === 'bigint') {
      return Number(value);
    }
    if (typeof value === 'object' && value !== null) {
      // Handle Codec types
      const codec = value as { toNumber?: () => number; toBigInt?: () => bigint };
      if (typeof codec.toNumber === 'function') {
        return codec.toNumber();
      }
      if (typeof codec.toBigInt === 'function') {
        return Number(codec.toBigInt());
      }
    }
    return Number(value);
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
      console.error('Max reconnection attempts exceeded for CC3');
      this.isRunning = false;
      return;
    }

    const delay = this.reconnectDelayMs * Math.pow(2, this.reconnectAttempts - 1);
    console.log(
      `⏳ Reconnecting to CC3 in ${delay}ms (attempt ${this.reconnectAttempts}/${this.maxReconnectAttempts})`,
    );

    // Clean up
    await this.cleanup();

    // Wait before reconnecting
    await new Promise((resolve) => setTimeout(resolve, delay));

    if (this.isRunning) {
      await this.connect();
    }
  }

  /**
   * Clean up resources
   */
  private async cleanup(): Promise<void> {
    if (this.unsubscribe) {
      try {
        this.unsubscribe();
      } catch {
        // Ignore
      }
      this.unsubscribe = null;
    }

    if (this.api) {
      try {
        await this.api.disconnect();
      } catch {
        // Ignore
      }
      this.api = null;
    }

    if (this.provider) {
      try {
        await this.provider.disconnect();
      } catch {
        // Ignore
      }
      this.provider = null;
    }
  }

  /**
   * Stop subscribing to events
   */
  async stop(): Promise<void> {
    console.log('⏹️  Stopping CC3 subscriber...');
    this.isRunning = false;
    await this.cleanup();
    console.log('✅ CC3 subscriber stopped');
  }

  /**
   * Check if connected
   */
  get isConnected(): boolean {
    return this.api !== null && this.api.isConnected && this.isRunning;
  }
}
