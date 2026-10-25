/**
 * Statistics collection and reporting for stress tests.
 *
 * Tracks request counts, latencies, status codes, and error codes.
 * Provides live console output and a final summary with percentiles.
 */

import type { RequestResult } from "./types.ts";

export class StatsCollector {
  private results: RequestResult[] = [];
  private startTime = 0;
  private liveInterval: ReturnType<typeof setInterval> | null = null;

  /** Start collecting and enable live reporting. */
  start(): void {
    this.startTime = performance.now();
    this.results = [];

    this.liveInterval = setInterval(() => {
      this.printLive();
    }, 2_000);
  }

  /** Record a completed request result. */
  record(result: RequestResult): void {
    this.results.push(result);
  }

  /** Get the start time (for external elapsed calculations). */
  getStartTime(): number {
    return this.startTime;
  }

  /** Get current number of recorded results. */
  getCount(): number {
    return this.results.length;
  }

  /** Get average latency of all recorded results. */
  getAverageLatency(): number {
    if (this.results.length === 0) return 0;
    const sum = this.results.reduce((acc, r) => acc + r.latencyMs, 0);
    return sum / this.results.length;
  }

  /** Stop live reporting. */
  stop(): void {
    if (this.liveInterval) {
      clearInterval(this.liveInterval);
      this.liveInterval = null;
    }
  }

  /** Print live status line. */
  private printLive(): void {
    const elapsed = (performance.now() - this.startTime) / 1000;
    const total = this.results.length;
    const ok = this.results.filter((r) => r.status >= 200 && r.status < 300)
      .length;
    const err = total - ok;
    const rps = total / elapsed;

    const latencies = this.results.map((r) => r.latencyMs).sort((a, b) =>
      a - b
    );
    const p50 = percentile(latencies, 0.5);
    const p99 = percentile(latencies, 0.99);

    const line = `[${
      elapsed.toFixed(0)
    }s] Sent: ${total} | OK: ${ok} | Err: ${err} | RPS: ${
      rps.toFixed(1)
    } | p50: ${p50.toFixed(0)}ms | p99: ${p99.toFixed(0)}ms`;

    // Overwrite the current line
    Deno.stdout.writeSync(new TextEncoder().encode(`\r\x1b[K${line}`));
  }

  /** Print the final summary report. */
  printSummary(): void {
    // Print newline to move past the live status line
    console.log("");

    const elapsed = (performance.now() - this.startTime) / 1000;
    const total = this.results.length;
    const ok = this.results.filter((r) => r.status >= 200 && r.status < 300)
      .length;
    const err = total - ok;

    const latencies = this.results.map((r) => r.latencyMs).sort((a, b) =>
      a - b
    );

    console.log("\n=== Stress Test Complete ===");
    console.log(`Duration:     ${elapsed.toFixed(1)}s`);
    console.log(`Total:        ${total} requests`);
    console.log(
      `Successful:   ${ok} (${
        total > 0 ? ((ok / total) * 100).toFixed(1) : 0
      }%)`,
    );
    console.log(
      `Failed:       ${err} (${
        total > 0 ? ((err / total) * 100).toFixed(1) : 0
      }%)`,
    );
    console.log(
      `Throughput:   ${(total / elapsed).toFixed(1)} req/s`,
    );

    if (latencies.length > 0) {
      console.log("\nLatency (ms):");
      console.log(
        `  p50: ${percentile(latencies, 0.5).toFixed(0)}    p90: ${
          percentile(latencies, 0.9).toFixed(0)
        }    p95: ${percentile(latencies, 0.95).toFixed(0)}    p99: ${
          percentile(latencies, 0.99).toFixed(0)
        }    max: ${latencies[latencies.length - 1].toFixed(0)}`,
      );
    }

    // Status code breakdown
    const statusCounts = new Map<number, number>();
    for (const r of this.results) {
      statusCounts.set(r.status, (statusCounts.get(r.status) ?? 0) + 1);
    }
    console.log("\nStatus codes:");
    for (const [status, count] of [...statusCounts.entries()].sort()) {
      const label = status === 0 ? "  0 (network error)" : `  ${status}`;
      console.log(`${label}: ${count}`);
    }

    // Error code breakdown
    const errorCounts = new Map<string, number>();
    for (const r of this.results) {
      if (r.errorCode) {
        errorCounts.set(
          r.errorCode,
          (errorCounts.get(r.errorCode) ?? 0) + 1,
        );
      }
    }
    if (errorCounts.size > 0) {
      console.log("\nError breakdown:");
      for (
        const [code, count] of [...errorCounts.entries()].sort(
          (a, b) => b[1] - a[1],
        )
      ) {
        console.log(`  ${code}: ${count}`);
      }
    }

    // Valid vs invalid breakdown
    const validOk =
      this.results.filter((r) =>
        r.kind === "valid" && r.status >= 200 && r.status < 300
      ).length;
    const validTotal = this.results.filter((r) => r.kind === "valid").length;
    const invalidTotal = this.results.filter((r) => r.kind === "invalid")
      .length;

    if (validTotal > 0 && invalidTotal > 0) {
      console.log("\nBy request type:");
      console.log(
        `  Valid:   ${validTotal} sent, ${validOk} succeeded (${
          ((validOk / validTotal) * 100).toFixed(1)
        }%)`,
      );
      console.log(
        `  Invalid: ${invalidTotal} sent`,
      );
    }

    console.log("");
  }
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.ceil(p * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}
