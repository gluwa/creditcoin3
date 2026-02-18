/**
 * Relayer client configuration.
 */

export interface RelayerConfig {
  /** RPC URL for the destination chain (where inbox is deployed) */
  rpcUrl: string;
  /** Private key for the relayer (pays gas) */
  privateKey: string;
  /** DummyInbox contract address */
  inboxAddress: string;
  /** Poll interval in ms when watching a messages file */
  pollIntervalMs: number;
  /** Path to JSON file with pending messages (mock P2P) */
  messagesFilePath: string;
  /** HTTP port for receiving messages (0 = disabled) */
  httpPort: number;
}

const DEFAULT_KEY = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

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
  const rpcUrl = parseArg("--rpc-url", "-r") ?? process.env.RELAYER_RPC_URL ?? "http://127.0.0.1:8545";
  const inbox = parseArg("--inbox", "-i") ?? process.env.RELAYER_INBOX_ADDRESS;
  const key = parseArg("--private-key", "-k") ?? process.env.RELAYER_PRIVATE_KEY ?? DEFAULT_KEY;
  const messagesFile =
    parseArg("--messages-file", "-m") ?? process.env.RELAYER_MESSAGES_FILE ?? "./messages.json";
  const httpPort = parseInt(
    parseArg("--http-port") ?? process.env.RELAYER_HTTP_PORT ?? "3301",
    10
  );
  const pollInterval = parseInt(
    process.env.RELAYER_POLL_INTERVAL_MS ?? "2000",
    10
  );

  // Try deployments.json if inbox not provided
  let inboxAddress = inbox;
  if (!inboxAddress) {
    try {
      const { readFile } = await import("fs/promises");
      const { existsSync } = await import("fs");
      const path = await import("path");
      const deployPath = path.join(process.cwd(), "deployments.json");
      if (existsSync(deployPath)) {
        const raw = await readFile(deployPath, "utf-8");
        const d = JSON.parse(raw);
        inboxAddress = d.inbox;
      }
    } catch {
      // ignore
    }
  }
  if (!inboxAddress) {
    throw new Error(
      "Missing inbox address. Pass --inbox 0x... or set RELAYER_INBOX_ADDRESS. Run 'npm run deploy' first."
    );
  }

  return {
    rpcUrl,
    privateKey: key.startsWith("0x") ? key : `0x${key}`,
    inboxAddress: inboxAddress!,
    pollIntervalMs: pollInterval,
    messagesFilePath: messagesFile,
    httpPort,
  };
}
