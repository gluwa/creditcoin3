/**
 * Attestation watcher for Creditcoin3
 *
 * Subscribes to BlockAttested and CheckpointReached events via WebSocket.
 */

import { ApiPromise, WsProvider } from "@polkadot/api";
import { BaseWatcher } from "./baseWatcher.ts";

export type AttestationCallback = (blockNumber: number) => void | Promise<void>;

export class AttestationWatcher extends BaseWatcher {
  protected readonly name = "CC3";
  private api: ApiPromise | null = null;
  private provider: WsProvider | null = null;
  private unsubscribe: (() => void) | null = null;
  // null = "never successfully processed a block" (uninitialized sentinel).
  // Using null instead of 0 separates "not started" from "last success was N"
  // so a failed first callback leaves lastFinalizedBlock as null, causing the
  // next callback to retry from the same startBlock rather than jumping to the
  // new tip and silently skipping all blocks in between.
  private lastFinalizedBlock: number | null = null;
  // Serializes async subscription callbacks so that concurrent invocations
  // cannot race on lastFinalizedBlock.  Each callback chains onto this promise
  // and the chain advances only after the previous callback fully completes.
  private callbackQueue: Promise<void> = Promise.resolve();
  private wsUrl: string;
  private chainKey: number;
  private onAttested: AttestationCallback;

  constructor(
    wsUrl: string,
    chainKey: number,
    onAttested: AttestationCallback,
  ) {
    super();
    this.wsUrl = wsUrl;
    this.chainKey = chainKey;
    this.onAttested = onAttested;
  }

  protected async connect(): Promise<void> {
    try {
      console.log(`🔗 Connecting to Creditcoin3: ${this.wsUrl}`);

      this.provider = new WsProvider(this.wsUrl);

      this.provider.on("error", async () => {
        console.error("CC3 WebSocket error");
        if (this.isRunning) await this.reconnect();
      });

      this.provider.on("disconnected", async () => {
        console.warn("CC3 WebSocket disconnected");
        if (this.isRunning) await this.reconnect();
      });

      this.api = await ApiPromise.create({
        provider: this.provider,
        noInitWarn: true,
      });
      await this.api.isReady;

      const chain = await this.api.rpc.system.chain();
      console.log(`✅ Connected to Creditcoin3 (${chain})`);
      this.resetReconnectAttempts();

      // Subscribe to finalized heads only, matching the proof-gen API which
      // waits for finalization before generating proofs.
      //
      // GRANDPA may finalize blocks in batches: subscribeFinalizedHeads only
      // emits the tip of each batch, so we backfill any skipped blocks to
      // avoid missing attestation events.
      this.lastFinalizedBlock = null;
      this.callbackQueue = Promise.resolve();
      this.unsubscribe =
        (await this.api.rpc.chain.subscribeFinalizedHeads((header) => {
          // Polkadot.js fires subscription callbacks in a fire-and-forget
          // fashion — the returned promise is never awaited.  Chain each
          // invocation onto callbackQueue so they execute sequentially and
          // lastFinalizedBlock is never read/written concurrently.
          this.callbackQueue = this.callbackQueue.then(async () => {
            const api = this.api;
            if (!api) return;

            const currentBlock = header.number.toNumber();
            // null → first callback ever (or after reconnect): start at
            // currentBlock.  A non-null value means we have at least one
            // success, so resume from the block after the last success.
            const startBlock = this.lastFinalizedBlock !== null
              ? this.lastFinalizedBlock + 1
              : currentBlock;

            let lastSuccess = this.lastFinalizedBlock;
            for (
              let blockNum = startBlock;
              blockNum <= currentBlock;
              blockNum++
            ) {
              try {
                const blockHash = await api.rpc.chain.getBlockHash(blockNum);
                const events = await api.query.system.events.at(blockHash);
                this.handleEvents(events as unknown);
                // Only advance the marker after a successful fetch so that
                // a transient RPC error does not permanently skip a block.
                lastSuccess = blockNum;
              } catch (error) {
                console.error(
                  `Error fetching finalized block events for block ${blockNum}:`,
                  error,
                );
                // Stop backfilling at the first failure so we retry from
                // this block on the next callback rather than skipping it.
                break;
              }
            }

            this.lastFinalizedBlock = lastSuccess;
          });
        })) as unknown as () => void;
    } catch (error) {
      console.error("Failed to connect to Creditcoin3:", error);
      if (this.isRunning) await this.reconnect();
    }
  }

  private handleEvents(events: unknown): void {
    const eventRecords = events as Array<{
      event: { section: string; method: string; data: unknown[] };
    }>;

    for (const { event } of eventRecords) {
      if (event.section === "attestation") {
        if (event.method === "BlockAttested") {
          this.handleBlockAttested(event.data);
        } else if (event.method === "CheckpointReached") {
          this.handleCheckpointReached(event.data);
        }
      }
    }
  }

  private handleBlockAttested(data: unknown[]): void {
    try {
      const eventChainKey = this.toNumber(data[0]);
      const headerNumber = this.toNumber(data[1]);

      if (eventChainKey === this.chainKey) {
        console.log(
          `📢 BlockAttested: block ${headerNumber} on chain ${eventChainKey}`,
        );
        this.onAttested(headerNumber);
      }
    } catch (error) {
      console.error("Error parsing BlockAttested event:", error);
    }
  }

  private handleCheckpointReached(data: unknown[]): void {
    try {
      const eventChainKey = this.toNumber(data[0]);
      const checkpoint = data[1] as {
        block_number?: number;
        blockNumber?: number;
      };
      const blockNumber = checkpoint.block_number ?? checkpoint.blockNumber;

      if (eventChainKey === this.chainKey && blockNumber !== undefined) {
        console.log(
          `📢 CheckpointReached: block ${blockNumber} on chain ${eventChainKey}`,
        );
        this.onAttested(blockNumber);
      }
    } catch (error) {
      console.error("Error parsing CheckpointReached event:", error);
    }
  }

  private toNumber(value: unknown): number {
    if (typeof value === "number") return value;
    if (typeof value === "bigint") return Number(value);
    if (typeof value === "object" && value !== null) {
      const codec = value as {
        toNumber?: () => number;
        toBigInt?: () => bigint;
      };
      if (typeof codec.toNumber === "function") return codec.toNumber();
      if (typeof codec.toBigInt === "function") return Number(codec.toBigInt());
    }
    return Number(value);
  }

  protected async cleanup(): Promise<void> {
    this.lastFinalizedBlock = null;
    this.callbackQueue = Promise.resolve();
    if (this.unsubscribe) {
      try {
        this.unsubscribe();
      } catch { /* ignore */ }
      this.unsubscribe = null;
    }
    if (this.api) {
      try {
        await this.api.disconnect();
      } catch { /* ignore */ }
      this.api = null;
    }
    if (this.provider) {
      try {
        await this.provider.disconnect();
      } catch { /* ignore */ }
      this.provider = null;
    }
  }

  get isConnected(): boolean {
    return this.api !== null && this.api.isConnected && this.isRunning;
  }
}
