/**
 * Attester configuration.
 * Uses CLI args first, then env vars, then defaults.
 *
 * Start: node dist/attester/server.js --outbox 0x... --private-key 0x...
 */

import {
  DEFAULT_POLL_INTERVAL_MS,
  DEFAULT_RELAYER_URL,
  DEFAULT_SOURCE_RPC_URL,
  DEPLOYMENTS_FILE,
} from "../consts.js";
import { isValidContractAddress, isValidPrivateKey } from "../utils.js";

export interface AttesterConfig {
  /** Source chain RPC (where Outbox lives) */
  sourceRpcUrl: string;
  /** Private key for message signing */
  key: string;
  /** Outbox contract address on source chain */
  outboxAddress: string;
  /** Relayer HTTP base URL (e.g. http://localhost:3301) */
  relayerUrl: string;
  /** Poll interval in ms for event queries */
  pollIntervalMs: number;
}

function parseArg(name: string, short?: string): string | undefined {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === name || (short && args[i] === short)) {
      return args[i + 1];
    }
    if (args[i].startsWith(`${name}=`)) {
      return args[i].slice(name.length + 1);
    }
  }
  return undefined;
}

export async function loadAttesterConfig(): Promise<AttesterConfig> {
  const sourceRpcUrl =
    parseArg("--source-rpc-url") ??
    process.env.ATTESTER_SOURCE_RPC_URL ??
    DEFAULT_SOURCE_RPC_URL;

  const key =
    parseArg("--private-key", "-k") ?? process.env.RELAYER_PRIVATE_KEY;

  const outbox =
    parseArg("--outbox", "-o") ?? process.env.ATTESTER_OUTBOX_ADDRESS;

  const relayerUrl =
    parseArg("--relayer-url") ??
    process.env.ATTESTER_RELAYER_URL ??
    DEFAULT_RELAYER_URL;

  const pollIntervalMs = parseInt(
    parseArg("--poll-interval") ??
      process.env.ATTESTER_POLL_INTERVAL_MS ??
      DEFAULT_POLL_INTERVAL_MS,
    10,
  );

  // Try deployments.json for addresses not provided
  let outboxAddress = outbox;
  if (!outboxAddress) {
    try {
      const { readFile } = await import("fs/promises");
      const { existsSync } = await import("fs");
      const path = await import("path");
      const deployPath = path.join(process.cwd(), DEPLOYMENTS_FILE);
      if (existsSync(deployPath)) {
        const raw = await readFile(deployPath, "utf-8");
        const d = JSON.parse(raw);
        if (!outboxAddress) outboxAddress = d.outbox;
      }
    } catch {
      // ignore
      console.error(
        `Failed to read ${DEPLOYMENTS_FILE} for contract addresses`,
      );
    }
  }

  if (!isValidContractAddress(outboxAddress)) {
    throw new Error(
      "Invalid or missing outbox address. Pass --outbox 0x<40 hex chars> or set ATTESTER_OUTBOX_ADDRESS.",
    );
  }

  if (!isValidPrivateKey(key)) {
    throw new Error(
      "Invalid or missing private key. Pass --private-key 0x<64 hex chars> or set RELAYER_PRIVATE_KEY.",
    );
  }

  return {
    sourceRpcUrl,
    key,
    outboxAddress,
    relayerUrl,
    pollIntervalMs,
  };
}
