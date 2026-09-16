// Registry chain keys (the same numeric key BridgeVault/BridgeHub/Outbox use — NOT the EVM chain
// id) for each spoke this dApp knows about. Base Sepolia's key is unset until its `register_chain`
// extrinsic sequence lands (blocked on asc-contracts#1319 per the bridge POC plan §4/§7) — routes
// with an unset chain key are hidden from the deposit form's destination picker rather than sent
// on-chain with a bogus key.
import { baseSepolia, sepolia } from "viem/chains";
import type { Chain } from "viem";

export interface SpokeChain {
  chainKey: number;
  chain: Chain;
  bridgeVaultAddress: `0x${string}`;
  /** Block `BridgeVault` was deployed at — the floor for History's log scan, so a fresh browser
   *  doesn't try to scan from block 0 across millions of blocks (see use-bridge-history.ts).
   *  Defaults to 0n (the old, slow-but-correct behavior) if unset. */
  genesisBlock: bigint;
  /** This spoke's shared DispatcherRouter address (the same one `BridgeVault` trusts as its
   *  Inbox) — optional, only used to detect a terminal `DestinationDeliveryFailed` delivery on the
   *  progress screen (see use-transfer-status.ts). Without it, a failed delivery just looks stuck. */
  dispatcherRouterAddress?: `0x${string}`;
  /** Overrides the chain's default public RPC (wired into wagmi's transports in wagmi.ts) — the
   *  default public RPCs from viem/chains turned out to have much stricter eth_getLogs block-range
   *  caps than assumed (Sepolia's default, thirdweb, caps at 1000 blocks and 403s non-browser
   *  requests entirely), which silently broke every chunked log scan in this app once a scan
   *  needed to look back further than that. Without this set, falls back to viem/chains' default. */
  rpcUrl?: string;
}

function optionalChainKey(raw: string | undefined): number | undefined {
  if (!raw) return undefined;
  const n = Number(raw);
  return Number.isInteger(n) && n > 0 ? n : undefined;
}

function optionalAddress(raw: string | undefined): `0x${string}` | undefined {
  if (!raw) return undefined;
  return raw as `0x${string}`;
}

function genesisBlock(raw: string | undefined): bigint {
  if (!raw) return 0n;
  try {
    const n = BigInt(raw);
    return n >= 0n ? n : 0n;
  } catch {
    return 0n;
  }
}

function buildSpokes(): SpokeChain[] {
  const spokes: SpokeChain[] = [];

  const ethSepoliaKey = optionalChainKey(
    process.env.NEXT_PUBLIC_ETH_SEPOLIA_CHAIN_KEY,
  );
  const ethSepoliaVault = optionalAddress(
    process.env.NEXT_PUBLIC_ETH_SEPOLIA_BRIDGE_VAULT_ADDRESS,
  );
  if (ethSepoliaKey && ethSepoliaVault) {
    spokes.push({
      chainKey: ethSepoliaKey,
      chain: sepolia,
      bridgeVaultAddress: ethSepoliaVault,
      genesisBlock: genesisBlock(
        process.env.NEXT_PUBLIC_ETH_SEPOLIA_VAULT_GENESIS_BLOCK,
      ),
      dispatcherRouterAddress: optionalAddress(
        process.env.NEXT_PUBLIC_ETH_SEPOLIA_ROUTER_ADDRESS,
      ),
      rpcUrl: process.env.NEXT_PUBLIC_ETH_SEPOLIA_RPC_URL || undefined,
    });
  }

  const baseSepoliaKey = optionalChainKey(
    process.env.NEXT_PUBLIC_BASE_SEPOLIA_CHAIN_KEY,
  );
  const baseSepoliaVault = optionalAddress(
    process.env.NEXT_PUBLIC_BASE_SEPOLIA_BRIDGE_VAULT_ADDRESS,
  );
  if (baseSepoliaKey && baseSepoliaVault) {
    spokes.push({
      chainKey: baseSepoliaKey,
      chain: baseSepolia,
      bridgeVaultAddress: baseSepoliaVault,
      genesisBlock: genesisBlock(
        process.env.NEXT_PUBLIC_BASE_SEPOLIA_VAULT_GENESIS_BLOCK,
      ),
      dispatcherRouterAddress: optionalAddress(
        process.env.NEXT_PUBLIC_BASE_SEPOLIA_ROUTER_ADDRESS,
      ),
      rpcUrl: process.env.NEXT_PUBLIC_BASE_SEPOLIA_RPC_URL || undefined,
    });
  }

  return spokes;
}

export const SPOKE_CHAINS: SpokeChain[] = buildSpokes();

export function spokeByChainKey(chainKey: number): SpokeChain | undefined {
  return SPOKE_CHAINS.find((s) => s.chainKey === chainKey);
}

export function spokeByEvmChainId(evmChainId: number): SpokeChain | undefined {
  return SPOKE_CHAINS.find((s) => s.chain.id === evmChainId);
}

// wagmi/RainbowKit require a non-empty tuple; fall back to Sepolia alone if no spoke is configured
// yet (e.g. a fresh checkout before .env.local is populated from usc-bridge-deploy.json).
export const WAGMI_CHAINS: readonly [Chain, ...Chain[]] =
  SPOKE_CHAINS.length > 0
    ? (SPOKE_CHAINS.map((s) => s.chain) as [Chain, ...Chain[]])
    : [sepolia];
