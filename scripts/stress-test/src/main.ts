/**
 * Proof-Gen API Stress Test
 *
 * Floods the proof-gen API with configurable volumes of valid, invalid,
 * and mixed requests at controlled rates.
 */

import { loadConfig, logConfig } from "./config.ts";
import { fetchBlocks } from "./blockFetcher.ts";
import { generateValidRequests } from "./generators/valid.ts";
import { generateInvalidRequests } from "./generators/invalid.ts";
import { generateMixedRequests } from "./generators/mixed.ts";
import { runStressTest } from "./runner.ts";
import type { StressRequest } from "./types.ts";

async function main(): Promise<void> {
  const config = loadConfig();
  logConfig(config);

  // Pre-calculate how many requests we'll need in the pool
  // Pool should be large enough to avoid excessive repetition
  const totalExpected = config.rps * config.duration;
  const poolSize = Math.min(totalExpected, 10_000);

  let requests: StressRequest[];

  switch (config.mode) {
    case "valid": {
      if (!config.sourceRpcUrl) {
        throw new Error("Source RPC URL is required for valid mode");
      }
      const blocks = await fetchBlocks(
        config.sourceRpcUrl,
        200,
        config.blockRange,
      );
      requests = generateValidRequests(
        config.apiUrl,
        config.chainKey,
        blocks,
        poolSize,
      );
      break;
    }

    case "invalid": {
      requests = generateInvalidRequests(
        config.apiUrl,
        config.chainKey,
        poolSize,
      );
      break;
    }

    case "mixed": {
      if (!config.sourceRpcUrl) {
        throw new Error("Source RPC URL is required for mixed mode");
      }
      const blocks = await fetchBlocks(
        config.sourceRpcUrl,
        200,
        config.blockRange,
      );
      requests = generateMixedRequests(
        config.apiUrl,
        config.chainKey,
        blocks,
        poolSize,
        config.mixRatio,
      );
      break;
    }
  }

  console.log(`Generated ${requests.length} request URLs`);
  await runStressTest(requests, config);
}

main().catch((err) => {
  console.error("Fatal error:", err.message ?? err);
  Deno.exit(1);
});
