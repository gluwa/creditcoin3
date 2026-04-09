/**
 * Base watcher class with common reconnection logic
 */

import { MAX_RECONNECT_ATTEMPTS } from "../constants.ts";
import {
  getReconnectDelay,
  logReconnectAttempt,
  logReconnectFailed,
} from "../utils/backoff.ts";
import { sleep } from "../utils/sleep.ts";

export abstract class BaseWatcher {
  protected isRunning = false;
  protected reconnectAttempts = 0;
  protected abstract readonly name: string;

  async start(): Promise<void> {
    if (this.isRunning) return;
    this.isRunning = true;
    await this.connect();
  }

  protected abstract connect(): Promise<void>;
  protected abstract cleanup(): Promise<void>;
  abstract get isConnected(): boolean;

  protected resetReconnectAttempts(): void {
    this.reconnectAttempts = 0;
  }

  protected async reconnect(): Promise<void> {
    if (!this.isRunning) return;

    this.reconnectAttempts++;
    if (this.reconnectAttempts > MAX_RECONNECT_ATTEMPTS) {
      logReconnectFailed(this.name);
      this.isRunning = false;
      return;
    }

    const delay = getReconnectDelay(this.reconnectAttempts);
    logReconnectAttempt(this.name, this.reconnectAttempts, delay);

    await this.cleanup();
    await sleep(delay);

    if (this.isRunning) await this.connect();
  }

  async stop(): Promise<void> {
    console.log(`⏹️  Stopping ${this.name} watcher...`);
    this.isRunning = false;
    await this.cleanup();
    console.log(`✅ ${this.name} watcher stopped`);
  }
}
