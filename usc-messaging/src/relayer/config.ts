/**
 * Relayer client configuration.
 *
 * Start: node dist/relayer/server.js --inbox 0x... --outbox 0x...
 */

import dotenv from "dotenv";

import {
  DEFAULT_DESTINATION_RPC_URL,
  DEFAULT_RELAYER_HTTP_PORT,
  DEFAULT_SOURCE_RPC_URL,
  DEPLOYMENTS_FILE,
} from "../consts.js";
import { isValidContractAddress, isValidPrivateKey } from "../utils.js";

const DEFAULT_DELIVERY_INTERVAL_MS = "5000";

export interface RelayerConfig {
  /** RPC URL for the destination chain (where SimpleInbox is deployed) */
  rpcUrl: string;
  /** Private key for the relayer (pays gas on both chains) */
  privateKey: string;
  /** SimpleInbox contract address on destination chain */
  inboxAddress: string;
  /** RPC URL for the source chain (where Outbox is deployed, used for ACK) */
  sourceRpcUrl: string;
  /** Outbox contract address on source chain (used for ACK) */
  outboxAddress: string;
  /** How often (ms) the delivery worker attempts to process the pending queue */
  deliveryIntervalMs: number;
  /** HTTP port for receiving messages from attesters (0 = disabled) */
  httpPort: number;
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

export async function loadRelayerConfig(): Promise<RelayerConfig> {
  dotenv.config({ override: true });

  const rpcUrl =
    parseArg("--rpc-url", "-r") ??
    process.env.RELAYER_RPC_URL ??
    DEFAULT_DESTINATION_RPC_URL;

  const sourceRpcUrl =
    parseArg("--source-rpc-url") ??
    process.env.RELAYER_SOURCE_RPC_URL ??
    DEFAULT_SOURCE_RPC_URL;

  const key =
    parseArg("--private-key", "-k") ?? process.env.RELAYER_PRIVATE_KEY;

  const inbox = parseArg("--inbox", "-i") ?? process.env.RELAYER_INBOX_ADDRESS;
  const outbox =
    parseArg("--outbox", "-o") ?? process.env.RELAYER_OUTBOX_ADDRESS;

  const httpPort = parseInt(
    parseArg("--http-port") ??
      process.env.RELAYER_HTTP_PORT ??
      DEFAULT_RELAYER_HTTP_PORT,
    10,
  );

  const deliveryIntervalMs = parseInt(
    parseArg("--delivery-interval") ??
      process.env.RELAYER_DELIVERY_INTERVAL_MS ??
      DEFAULT_DELIVERY_INTERVAL_MS,
    10,
  );

  // Resolve inbox and outbox addresses: CLI/env first, then deployments.json fallback.
  let inboxAddress = inbox;
  let outboxAddress = outbox;
  if (!inboxAddress || !outboxAddress) {
    try {
      const { readFile } = await import("fs/promises");
      const { existsSync } = await import("fs");
      const path = await import("path");
      const deployPath = path.join(process.cwd(), DEPLOYMENTS_FILE);
      if (existsSync(deployPath)) {
        const raw = await readFile(deployPath, "utf-8");
        const d = JSON.parse(raw);
        if (!inboxAddress) inboxAddress = d.inbox;
        if (!outboxAddress) outboxAddress = d.outbox;
      }
    } catch {
      // ignore
    }
  }

  if (!isValidPrivateKey(key)) {
    throw new Error(
      "Invalid or missing private key. Pass --private-key 0x<64 hex chars> or set RELAYER_PRIVATE_KEY.",
    );
  }

  if (!isValidContractAddress(inboxAddress)) {
    throw new Error(
      "Invalid or missing inbox address. Pass --inbox 0x<40 hex chars> or set RELAYER_INBOX_ADDRESS.",
    );
  }

  if (!isValidContractAddress(outboxAddress)) {
    throw new Error(
      "Invalid or missing outbox address. Pass --outbox 0x<40 hex chars> or set RELAYER_OUTBOX_ADDRESS.",
    );
  }

  return {
    rpcUrl,
    sourceRpcUrl,
    privateKey: key.startsWith("0x") ? key : `0x${key}`,
    inboxAddress,
    outboxAddress,
    deliveryIntervalMs,
    httpPort,
  };
}
