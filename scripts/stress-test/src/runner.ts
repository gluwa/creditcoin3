/**
 * Core stress test orchestrator.
 *
 * Rate-limits and concurrency-limits request execution.
 * Uses a simple interval-based approach: fires a batch of requests every
 * 100ms (10 ticks/second), distributing the target RPS evenly.
 */

import type { StressConfig, StressRequest } from "./types.ts";
import { executeRequest } from "./requestor.ts";
import { StatsCollector } from "./stats.ts";

const TICK_INTERVAL_MS = 100;

/**
 * Run the stress test with the given requests and config.
 */
export function runStressTest(
  requests: StressRequest[],
  config: StressConfig,
): Promise<void> {
  const stats = new StatsCollector();
  const requestsPerTick = Math.max(1, Math.round(config.rps / 10));
  const endTime = Date.now() + config.duration * 1000;

  let requestIndex = 0;
  let activeCount = 0;
  let throttleWarned = false;
  const pending: Promise<void>[] = [];

  console.log(
    `Starting stress test: ${config.rps} rps, ${config.concurrency} concurrency, ${config.duration}s duration`,
  );
  console.log(`Request pool size: ${requests.length}\n`);

  stats.start();

  return new Promise<void>((resolve) => {
    // After 5 seconds, check if actual throughput is well below target
    const throttleCheck = setTimeout(() => {
      const elapsed = (performance.now() - stats.getStartTime()) / 1000;
      if (elapsed < 3) return;
      const actualRps = stats.getCount() / elapsed;
      if (actualRps < config.rps * 0.5) {
        const avgLatency = stats.getAverageLatency();
        const suggestedConcurrency = Math.ceil(config.rps * avgLatency / 1000);
        console.log(
          `\nWARNING: Actual throughput (${
            actualRps.toFixed(1)
          } rps) is well below target (${config.rps} rps).`,
        );
        console.log(
          `  Average response latency is ${
            avgLatency.toFixed(0)
          }ms, which requires ~${suggestedConcurrency} concurrent connections to sustain ${config.rps} rps.`,
        );
        console.log(
          `  Current concurrency limit is ${config.concurrency}. Consider: --concurrency ${suggestedConcurrency}\n`,
        );
        throttleWarned = true;
      }
    }, 5_000);

    const tick = setInterval(() => {
      if (Date.now() >= endTime) {
        clearInterval(tick);
        clearTimeout(throttleCheck);
        Promise.allSettled(pending).then(() => {
          stats.stop();
          stats.printSummary();
          if (throttleWarned) {
            const avgLatency = stats.getAverageLatency();
            const suggestedConcurrency = Math.ceil(
              config.rps * avgLatency / 1000,
            );
            console.log(
              `TIP: To reach ${config.rps} rps, try: --concurrency ${suggestedConcurrency}\n`,
            );
          }
          resolve();
        });
        return;
      }

      for (
        let i = 0;
        i < requestsPerTick && activeCount < config.concurrency;
        i++
      ) {
        const request = requests[requestIndex % requests.length];
        requestIndex++;
        activeCount++;

        const p = executeRequest(request, config.timeout).then((result) => {
          if (config.verbose) {
            const status = result.status === 0 ? "ERR" : String(result.status);
            const detail = result.errorCode ? ` [${result.errorCode}]` : "";
            console.log(
              `  ${request.kind.toUpperCase()} ${status} ${
                result.latencyMs.toFixed(0)
              }ms ${request.url}${detail}`,
            );
          }
          stats.record(result);
          activeCount--;
        }).catch(() => {
          activeCount--;
        });

        pending.push(p);
      }
    }, TICK_INTERVAL_MS);
  });
}
