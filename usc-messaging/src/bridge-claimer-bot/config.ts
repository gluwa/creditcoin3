// Configuration for the standalone bridge claimer bot: watches each spoke chain's BridgeVault for
// `Deposited` events and submits the matching `BridgeHub.claim` on Creditcoin. Deliberately its own
// service rather than an extension of asc-message-relayer's claim worker, which is hardcoded to one
// route with no chain-key parameter — see the bridge POC plan for the full rationale.
import "dotenv/config";

export interface RouteConfig {
  /** Registry chain key of this spoke (the same numeric key BridgeVault/BridgeHub/Outbox use), e.g.
   *  8 for Ethereum Sepolia. NOT an EVM chain id. */
  chainKey: number;
  /** Spoke chain RPC URL. */
  rpcUrl: string;
  bridgeVaultAddress: string;
  /** Blocks to hold back from the spoke chain's tip before treating a `Deposited` log as final
   *  enough to start proving — a reorg past this depth would need a manual re-scan (see README's
   *  "Known gaps" convention once this exists), not something this POC bot recovers from itself. */
  confirmationDepth: number;
}

export interface ClaimerConfig {
  routes: RouteConfig[];
  creditcoinRpcUrl: string;
  bridgeHubAddress: string;
  /** proof-gen-api-server base URL (the same service asc-message-relayer's ack path already uses). */
  proofGenUrl: string;
  /** Gas-funded, non-privileged Creditcoin EOA — `BridgeHub.claim` is fully permissionless, so this
   *  key only ever pays gas and never needs to be treated as sensitive beyond that. */
  claimSignerKey: string;
  pollIntervalMs: number;
}

function req(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing required env ${name}`);
  return v;
}

function loadRoutes(): RouteConfig[] {
  // JSON array: [{ "chainKey": 8, "rpcUrl": "...", "bridgeVaultAddress": "0x...", "confirmationDepth": 2 }]
  const raw = req("CLAIMER_ROUTES");
  const parsed = JSON.parse(raw) as Array<Record<string, unknown>>;
  if (parsed.length === 0)
    throw new Error("CLAIMER_ROUTES must list at least one route");
  return parsed.map((r) => {
    const chainKey = Number(r.chainKey);
    const rpcUrl = String(r.rpcUrl ?? "");
    const bridgeVaultAddress = String(r.bridgeVaultAddress ?? "");
    if (!Number.isInteger(chainKey) || chainKey <= 0) {
      throw new Error(`CLAIMER_ROUTES: invalid chainKey ${String(r.chainKey)}`);
    }
    if (!rpcUrl || !bridgeVaultAddress) {
      throw new Error(
        `CLAIMER_ROUTES: route for chainKey ${chainKey} missing rpcUrl/bridgeVaultAddress`,
      );
    }
    return {
      chainKey,
      rpcUrl,
      bridgeVaultAddress,
      confirmationDepth: Number(r.confirmationDepth ?? 2),
    };
  });
}

export function loadConfig(): ClaimerConfig {
  return {
    routes: loadRoutes(),
    creditcoinRpcUrl: req("CREDITCOIN_RPC_URL"),
    bridgeHubAddress: req("BRIDGE_HUB_ADDRESS"),
    proofGenUrl: req("PROOF_GEN_URL"),
    claimSignerKey: req("CLAIM_SIGNER_KEY"),
    pollIntervalMs: Number(process.env.CLAIMER_POLL_INTERVAL_MS ?? "5000"),
  };
}
