// Runtime config: loaded from /bridge-config.json (written by `npm run gen-config` or by hand),
// with per-field overrides persisted to localStorage via the in-app Settings panel. This keeps the
// (per-deploy) contract addresses out of the bundle.

export interface ChainConfig {
  name: string;
  chainId: number;
  metamaskRpcUrl: string; // direct URL MetaMask uses (add/switch network)
  /** Read-only RPC for ethers: a same-origin proxy path (`/rpc/cc`) or an absolute URL. */
  readRpcPath: string;
  /** Optional extra absolute URLs tried when `readRpcPath` fails — public gateways 403 some
   * viewer IPs (geo/VPN filtering), so no single endpoint works for everyone. */
  readRpcFallbacks?: string[];
  currencySymbol: string;
  token: string; // BridgeToken address on this chain
  bridge: string; // DestBridge / CcBridge address on this chain
}

export interface BridgeConfig {
  destChainKey: number;
  proofGenPath: string;
  chains: { creditcoin: ChainConfig; dest: ChainConfig };
}

const LS_KEY = "usc-bridge-overrides";

export type Overrides = Partial<{
  ccToken: string;
  ccBridge: string;
  destToken: string;
  destBridge: string;
}>;

export function loadOverrides(): Overrides {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

export function saveOverrides(o: Overrides) {
  localStorage.setItem(LS_KEY, JSON.stringify(o));
}

export async function loadConfig(): Promise<BridgeConfig> {
  const resp = await fetch("/bridge-config.json", { cache: "no-store" });
  const cfg = (await resp.json()) as BridgeConfig;
  const o = loadOverrides();
  if (o.ccToken) cfg.chains.creditcoin.token = o.ccToken;
  if (o.ccBridge) cfg.chains.creditcoin.bridge = o.ccBridge;
  if (o.destToken) cfg.chains.dest.token = o.destToken;
  if (o.destBridge) cfg.chains.dest.bridge = o.destBridge;
  return cfg;
}

export function isConfigured(cfg: BridgeConfig): boolean {
  const c = cfg.chains;
  return Boolean(c.creditcoin.token && c.creditcoin.bridge && c.dest.token && c.dest.bridge);
}
