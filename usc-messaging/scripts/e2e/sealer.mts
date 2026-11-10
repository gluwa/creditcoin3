import { setTimeout as delay } from "node:timers/promises";
import type { Rpc } from "./rpc.mjs";
import { FatalError } from "./runtime.mjs";

/**
 * Drives a `--sealing manual` Creditcoin node: one sealed and finalized block per tick. Block
 * time advances one slot per block however fast the ticks are, so `fastForward` crosses a
 * block-denominated timelock in seconds while the chain's own clock stays consistent.
 */
export class Sealer {
  private stopped = false;
  private loop?: Promise<void>;
  private intervalMs: number;
  private failure?: unknown;
  private hold?: number;
  private best = 0;
  sealed = 0;

  constructor(
    private readonly rpc: Rpc,
    private readonly steadyIntervalMs: number,
    private readonly signal: AbortSignal,
  ) {
    this.intervalMs = steadyIntervalMs;
  }

  start(): void {
    if (this.loop) throw new FatalError("sealer already started");
    this.loop = this.run();
  }

  assertAlive(): void {
    if (this.failure)
      throw new FatalError(`manual sealer stopped: ${String(this.failure)}`);
  }

  async height(): Promise<number> {
    return Number(BigInt(await this.rpc.call<string>("eth_blockNumber")));
  }

  /**
   * Seals at `blocksPerSecond` up to exactly `target` and stops there, so the caller can place
   * transactions in chosen blocks with `sealOne`. `release` resumes the steady pace.
   */
  async holdAt(target: number, blocksPerSecond: number): Promise<void> {
    if (!(blocksPerSecond > 0)) throw new FatalError("invalid sealing pace");
    if (this.best > target)
      throw new FatalError(`chain is already past block ${target}`);
    this.hold = target;
    this.intervalMs = 1000 / blocksPerSecond;
    while (this.best < target || this.sealing) {
      this.signal.throwIfAborted();
      this.assertAlive();
      await delay(50, undefined, { signal: this.signal });
    }
    // The EVM view of the head can trail the sealed block by a moment.
    let height = await this.height();
    while (height < target) {
      this.signal.throwIfAborted();
      await delay(50, undefined, { signal: this.signal });
      height = await this.height();
    }
    if (this.best !== target || height !== target)
      throw new FatalError(`sealer overshot block ${target}`);
  }

  /** Stops sealing at the current head, for `sealOne`; returns that head. */
  async pause(): Promise<number> {
    this.hold = this.best;
    while (this.sealing) await delay(20, undefined, { signal: this.signal });
    this.hold = this.best;
    return this.best;
  }

  /** Seals and finalizes one block while held; returns its number. */
  async sealOne(): Promise<number> {
    if (this.hold === undefined || this.sealing)
      throw new FatalError("sealOne needs a held sealer");
    await this.seal();
    this.hold = this.best;
    return this.best;
  }

  release(): void {
    this.hold = undefined;
    this.intervalMs = this.steadyIntervalMs;
  }

  /** Seals at `blocksPerSecond` until the best block reaches `target`, then resumes the steady pace. */
  async fastForward(target: number, blocksPerSecond: number): Promise<number> {
    if (!(blocksPerSecond > 0)) throw new FatalError("invalid sealing pace");
    this.intervalMs = 1000 / blocksPerSecond;
    try {
      for (;;) {
        this.signal.throwIfAborted();
        this.assertAlive();
        const height = await this.height();
        if (height >= target) return height;
        await delay(200, undefined, { signal: this.signal });
      }
    } finally {
      this.intervalMs = this.steadyIntervalMs;
    }
  }

  async stop(): Promise<void> {
    this.stopped = true;
    await this.loop;
  }

  private sealing = false;

  private async seal(): Promise<void> {
    this.sealing = true;
    try {
      const { hash } = await this.rpc.call<{ hash: string }>(
        "engine_createBlock",
        [true, false, null],
      );
      await this.rpc.call("engine_finalizeBlock", [hash, null]);
      this.sealed += 1;
      this.best += 1;
    } finally {
      this.sealing = false;
    }
  }

  private async run(): Promise<void> {
    try {
      this.best = await this.height();
    } catch (error) {
      this.failure = error;
      return;
    }
    while (!this.stopped && !this.signal.aborted) {
      const started = Date.now();
      if (this.hold !== undefined && this.best >= this.hold) {
        await delay(20);
        continue;
      }
      try {
        await this.seal();
      } catch (error) {
        // A stopped chain is fatal for every waiter; surface it through `assertAlive`.
        if (!this.stopped && !this.signal.aborted) this.failure = error;
        return;
      }
      const rest = this.intervalMs - (Date.now() - started);
      if (rest > 0) await delay(rest);
    }
  }
}
