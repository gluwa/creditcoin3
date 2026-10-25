/**
 * Configuration loading for the stress test tool
 */

import { parseArgs } from "jsr:@std/cli@^1.0.0/parse-args";
import type { StressConfig, StressMode } from "./types.ts";

const DEFAULTS = {
  rps: 50,
  concurrency: 20,
  duration: 60,
  mixRatio: 0.5,
  timeout: 60,
};

function printHelp(): void {
  console.log(`
Proof-Gen API Stress Test

Floods the proof-gen API with valid, invalid, or mixed requests
at configurable rates to test performance under load.

USAGE:
  deno task start [OPTIONS]

OPTIONS:
  -m, --mode <MODE>         Test mode: valid, invalid, mixed (required)
  -a, --api-url <URL>       Proof-gen API URL (required)
                            Env: API_URL
  -c, --chain-key <NUM>     Chain key (required)
                            Env: CHAIN_KEY
  -s, --source-rpc <URL>    Source chain HTTP RPC URL (required for valid/mixed)
                            Env: SOURCE_RPC_URL

      --rps <NUM>           Target requests/second (default: ${DEFAULTS.rps})
                            Env: RPS
      --concurrency <NUM>   Max concurrent requests (default: ${DEFAULTS.concurrency})
                            Env: CONCURRENCY
      --duration <SEC>      Test duration in seconds (default: ${DEFAULTS.duration})
                            Env: DURATION
      --mix-ratio <0-1>     Ratio of valid requests in mixed mode (default: ${DEFAULTS.mixRatio})
                            Env: MIX_RATIO
      --timeout <SEC>        Request timeout in seconds (default: ${DEFAULTS.timeout})
                            Env: TIMEOUT
      --block-range <S-E>   Block range for valid requests (e.g. 1000-2000)
                            Env: BLOCK_RANGE

  -v, --verbose             Log each individual request and response
                            Env: VERBOSE

  -h, --help                Show this help message

EXAMPLES:
  # Valid requests only at 100 rps
  deno task start -- -m valid -a http://localhost:3100 -c 1 -s https://sepolia.infura.io/v3/KEY --rps 100

  # Invalid requests only (no source RPC needed)
  deno task start -- -m invalid -a http://localhost:3100 -c 1

  # Mixed 70/30 valid/invalid
  deno task start -- -m mixed -a http://localhost:3100 -c 1 -s https://sepolia.infura.io/v3/KEY --mix-ratio 0.7
`);
}

function getString(
  args: Record<string, unknown>,
  argName: string,
  envName: string,
  defaultValue?: string,
): string {
  const argValue = args[argName];
  if (typeof argValue === "string" && argValue.length > 0) return argValue;

  const envValue = Deno.env.get(envName);
  if (envValue && envValue.length > 0) return envValue;

  if (defaultValue !== undefined) return defaultValue;

  throw new Error(
    `Missing required configuration: --${argName} or ${envName}`,
  );
}

function getNumber(
  args: Record<string, unknown>,
  argName: string,
  envName: string,
  defaultValue: number,
): number {
  const argValue = args[argName];
  if (typeof argValue === "string" || typeof argValue === "number") {
    const num = Number(argValue);
    if (!isNaN(num)) return num;
  }

  const envValue = Deno.env.get(envName);
  if (envValue) {
    const num = Number(envValue);
    if (!isNaN(num)) return num;
  }

  return defaultValue;
}

function parseBlockRange(value: string): [number, number] {
  const parts = value.split("-").map(Number);
  if (parts.length !== 2 || isNaN(parts[0]) || isNaN(parts[1])) {
    throw new Error(
      `Invalid block range: ${value} (expected format: START-END)`,
    );
  }
  if (parts[0] > parts[1]) {
    throw new Error(
      `Invalid block range: start (${parts[0]}) > end (${parts[1]})`,
    );
  }
  return [parts[0], parts[1]];
}

export function loadConfig(): StressConfig {
  const args = parseArgs(Deno.args, {
    string: [
      "mode",
      "api-url",
      "chain-key",
      "source-rpc",
      "rps",
      "concurrency",
      "duration",
      "mix-ratio",
      "timeout",
      "block-range",
    ],
    boolean: ["help", "verbose"],
    alias: {
      m: "mode",
      a: "api-url",
      c: "chain-key",
      s: "source-rpc",
      v: "verbose",
      h: "help",
    },
  });

  if (args.help) {
    printHelp();
    Deno.exit(0);
  }

  const mode = getString(args, "mode", "MODE") as StressMode;
  if (!["valid", "invalid", "mixed"].includes(mode)) {
    throw new Error(`Invalid mode: ${mode}. Must be valid, invalid, or mixed`);
  }

  const apiUrl = getString(args, "api-url", "API_URL").replace(/\/+$/, "");
  const chainKey = getNumber(args, "chain-key", "CHAIN_KEY", 0);
  if (chainKey === 0) {
    throw new Error("Chain key is required: --chain-key or CHAIN_KEY");
  }

  let sourceRpcUrl: string | undefined;
  if (mode !== "invalid") {
    sourceRpcUrl = getString(args, "source-rpc", "SOURCE_RPC_URL");
  } else {
    try {
      sourceRpcUrl = getString(args, "source-rpc", "SOURCE_RPC_URL");
    } catch {
      // Not required for invalid mode
    }
  }

  const rps = getNumber(args, "rps", "RPS", DEFAULTS.rps);
  const concurrency = getNumber(
    args,
    "concurrency",
    "CONCURRENCY",
    DEFAULTS.concurrency,
  );
  const duration = getNumber(args, "duration", "DURATION", DEFAULTS.duration);
  const mixRatio = getNumber(
    args,
    "mix-ratio",
    "MIX_RATIO",
    DEFAULTS.mixRatio,
  );

  let blockRange: [number, number] | undefined;
  const blockRangeStr = args["block-range"] as string ??
    Deno.env.get("BLOCK_RANGE");
  if (blockRangeStr) {
    blockRange = parseBlockRange(blockRangeStr);
  }

  // Validate
  if (rps <= 0) throw new Error("RPS must be positive");
  if (concurrency <= 0) throw new Error("Concurrency must be positive");
  if (duration <= 0) throw new Error("Duration must be positive");
  if (mixRatio < 0 || mixRatio > 1) {
    throw new Error("Mix ratio must be between 0.0 and 1.0");
  }

  const timeout = getNumber(args, "timeout", "TIMEOUT", DEFAULTS.timeout);
  if (timeout <= 0) throw new Error("Timeout must be positive");

  const verbose = args.verbose === true ||
    Deno.env.get("VERBOSE")?.toLowerCase() === "true";

  return {
    mode,
    apiUrl,
    chainKey,
    sourceRpcUrl,
    rps,
    concurrency,
    duration,
    mixRatio,
    blockRange,
    timeout: timeout * 1000, // convert to ms
    verbose,
  };
}

export function logConfig(config: StressConfig): void {
  console.log("\nConfiguration:");
  console.log(`  Mode:        ${config.mode}`);
  console.log(`  API URL:     ${config.apiUrl}`);
  console.log(`  Chain key:   ${config.chainKey}`);
  if (config.sourceRpcUrl) {
    console.log(`  Source RPC:  ${config.sourceRpcUrl}`);
  }
  console.log(`  RPS:         ${config.rps}`);
  console.log(`  Concurrency: ${config.concurrency}`);
  console.log(`  Duration:    ${config.duration}s`);
  console.log(`  Timeout:     ${config.timeout / 1000}s`);
  if (config.mode === "mixed") {
    console.log(`  Mix ratio:   ${config.mixRatio} (valid)`);
  }
  if (config.blockRange) {
    console.log(
      `  Block range: ${config.blockRange[0]}-${config.blockRange[1]}`,
    );
  }
  if (config.verbose) {
    console.log(`  Verbose:     enabled`);
  }
  console.log("");
}
