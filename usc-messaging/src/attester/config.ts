/**
 * Attester configuration.
 * Uses CLI args first, then env vars, then defaults.
 *
 * Start: node dist/attester/server.js --outbox 0x... --inbox 0x...
 */

// Environment variable names
const ENV_SOURCE_RPC_URL = "ATTESTER_SOURCE_RPC_URL";
const ENV_DESTINATION_RPC_URL = "ATTESTER_DESTINATION_RPC_URL";
const ENV_OUTBOX_ADDRESS = "ATTESTER_OUTBOX_ADDRESS";
const ENV_INBOX_ADDRESS = "ATTESTER_INBOX_ADDRESS";
const ENV_RELAYER_URL = "ATTESTER_RELAYER_URL";
const ENV_POLL_INTERVAL_MS = "ATTESTER_POLL_INTERVAL_MS";

// Default values
const DEFAULT_SOURCE_RPC_URL = "http://127.0.0.1:9944";
const DEFAULT_DESTINATION_RPC_URL = "http://127.0.0.1:8545";
const DEFAULT_RELAYER_URL = "http://127.0.0.1:3301";
const DEFAULT_POLL_INTERVAL_MS = "5000";
const DEPLOYMENTS_FILE = "deployments.json";

export interface AttesterConfig {
  /** Source chain RPC (where Outbox lives) */
  sourceRpcUrl: string;
  /** Destination chain RPC (where Inbox lives) */
  destinationRpcUrl: string;
  /** Outbox contract address on source chain */
  outboxAddress: string;
  /** Inbox contract address on destination chain */
  inboxAddress: string;
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
    process.env[ENV_SOURCE_RPC_URL] ??
    DEFAULT_SOURCE_RPC_URL;

  const destinationRpcUrl =
    parseArg("--destination-rpc-url") ??
    process.env[ENV_DESTINATION_RPC_URL] ??
    DEFAULT_DESTINATION_RPC_URL;

  const outbox = parseArg("--outbox", "-o") ?? process.env[ENV_OUTBOX_ADDRESS];
  const inbox = parseArg("--inbox", "-i") ?? process.env[ENV_INBOX_ADDRESS];

  const relayerUrl =
    parseArg("--relayer-url") ??
    process.env[ENV_RELAYER_URL] ??
    DEFAULT_RELAYER_URL;

  const pollIntervalMs = parseInt(
    parseArg("--poll-interval") ??
      process.env[ENV_POLL_INTERVAL_MS] ??
      DEFAULT_POLL_INTERVAL_MS,
    10,
  );

  // Try deployments.json for addresses not provided
  let outboxAddress = outbox;
  let inboxAddress = inbox;
  if (!outboxAddress || !inboxAddress) {
    try {
      const { readFile } = await import("fs/promises");
      const { existsSync } = await import("fs");
      const path = await import("path");
      const deployPath = path.join(process.cwd(), DEPLOYMENTS_FILE);
      if (existsSync(deployPath)) {
        const raw = await readFile(deployPath, "utf-8");
        const d = JSON.parse(raw);
        if (!outboxAddress) outboxAddress = d.outbox;
        if (!inboxAddress) inboxAddress = d.inbox;
      }
    } catch {
      // ignore
      console.error(
        `Failed to read ${DEPLOYMENTS_FILE} for contract addresses`,
      );
    }
  }

  if (!outboxAddress) {
    throw new Error(
      "Missing outbox address. Pass --outbox 0x... or set ATTESTER_OUTBOX_ADDRESS.",
    );
  }
  if (!inboxAddress) {
    throw new Error(
      "Missing inbox address. Pass --inbox 0x... or set ATTESTER_INBOX_ADDRESS.",
    );
  }

  return {
    sourceRpcUrl,
    destinationRpcUrl,
    outboxAddress,
    inboxAddress,
    relayerUrl,
    pollIntervalMs,
  };
}
