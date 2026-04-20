/**
 * Attester configuration.
 * Uses CLI args first, then env vars, then defaults.
 *
 * Start: node dist/attester/server.js --outbox 0x... --private-key 0x...
 */

import dotenv from "dotenv";

import {
  DEFAULT_POLL_INTERVAL_MS,
  DEFAULT_RELAYER_URL,
  DEFAULT_SOURCE_RPC_URL,
  DEPLOYMENTS_FILE,
} from "../consts.js";
import { isValidContractAddress, isValidPrivateKey, parseArg } from "../utils.js";

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

export async function loadAttesterConfig(): Promise<AttesterConfig> {
  dotenv.config({ override: true });

  const sourceRpcUrl =
    parseArg("--source-rpc-url") ??
    process.env.CREDITCOIN_RPC_URL ??
    DEFAULT_SOURCE_RPC_URL;

  const key =
    parseArg("--private-key", "-k") ??
    process.env.DESTINATION_CHAIN_PRIVATE_KEY;

  const outbox = parseArg("--outbox", "-o") ?? process.env.OUTBOX_ADDR;

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
      "Invalid or missing outbox address. Pass --outbox 0x<40 hex chars> or set OUTBOX_ADDR.",
    );
  }

  if (!isValidPrivateKey(key)) {
    throw new Error(
      "Invalid or missing private key. Pass --private-key 0x<64 hex chars> or set DESTINATION_CHAIN_PRIVATE_KEY.",
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
