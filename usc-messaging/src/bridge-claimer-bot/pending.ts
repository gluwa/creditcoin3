// In-memory dedup + backoff bookkeeping for deposits currently being claimed.
//
// ⚠️ POC scope, matching dApp-ack-worker's own disclaimer: this state is NOT crash-durable. A
// restart forgets in-flight/backoff state and re-derives it from the watcher's lookback rewind and
// BridgeHub.claimed's on-chain truth — see claim.ts's pre-flight check, which is the real
// backstop against double-submission, not this tracker. This tracker only avoids redundant work
// (concurrent duplicate proof fetches, tight retry loops), never correctness.

const INITIAL_BACKOFF_MS = 5_000;
const MAX_BACKOFF_MS = 5 * 60_000;

interface DepositState {
  inFlight: boolean;
  claimed: boolean;
  failureCount: number;
  nextRetryAt: number;
}

export class ClaimTracker {
  private readonly deposits = new Map<string, DepositState>();

  private key(chainKey: number, nonce: bigint): string {
    return `${chainKey}:${nonce}`;
  }

  private stateFor(chainKey: number, nonce: bigint): DepositState {
    const k = this.key(chainKey, nonce);
    let s = this.deposits.get(k);
    if (!s) {
      s = { inFlight: false, claimed: false, failureCount: 0, nextRetryAt: 0 };
      this.deposits.set(k, s);
    }
    return s;
  }

  /** True if this deposit is already claimed, in flight, or in backoff — the caller should skip it. */
  shouldSkip(chainKey: number, nonce: bigint): boolean {
    const s = this.stateFor(chainKey, nonce);
    if (s.claimed || s.inFlight) return true;
    return Date.now() < s.nextRetryAt;
  }

  markInFlight(chainKey: number, nonce: bigint): void {
    this.stateFor(chainKey, nonce).inFlight = true;
  }

  recordSuccess(chainKey: number, nonce: bigint): void {
    const s = this.stateFor(chainKey, nonce);
    s.inFlight = false;
    s.claimed = true;
  }

  /** Marks terminal (a decoded on-chain revert that will never succeed on retry, e.g. already
   *  claimed by a racing submitter, or the source/destination chain was disabled). */
  recordTerminal(chainKey: number, nonce: bigint): void {
    const s = this.stateFor(chainKey, nonce);
    s.inFlight = false;
    s.claimed = true;
  }

  /** Marks transient (RPC hiccup, proof not ready yet, nonce race): backs off exponentially,
   *  capped, and left retryable — never a terminal give-up on its own. */
  recordTransientFailure(chainKey: number, nonce: bigint): void {
    const s = this.stateFor(chainKey, nonce);
    s.inFlight = false;
    s.failureCount += 1;
    const backoff = Math.min(
      INITIAL_BACKOFF_MS * 2 ** (s.failureCount - 1),
      MAX_BACKOFF_MS,
    );
    s.nextRetryAt = Date.now() + backoff;
  }
}
