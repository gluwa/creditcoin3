// Read-only state for the Inspect page — plain contract reads, no writes. BridgeHub lives on
// Creditcoin (never a wagmi chain, see lib/contracts/hub.ts), so it's read with the same raw
// viem client used elsewhere for the "claimed" step; each BridgeVault lives on a spoke chain
// that *is* a wagmi chain, read via wagmi's plain getPublicClient action (dynamic per spoke, so
// this can't just be the usePublicClient hook — same reasoning as use-bridge-history.ts).
import { useConfig } from "wagmi";
import { getPublicClient } from "wagmi/actions";
import { useQuery } from "@tanstack/react-query";

import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import { BridgeHubAbi } from "@/lib/contracts/generated/bridge-hub.abi";
import { SPOKE_CHAINS, type SpokeChain } from "@/lib/chains";
import {
  BRIDGE_HUB_ADDRESS,
  creditcoinPublicClient,
} from "@/lib/contracts/hub";

const ERC20_ABI = [
  {
    type: "function",
    name: "symbol",
    inputs: [],
    outputs: [{ type: "string" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "decimals",
    inputs: [],
    outputs: [{ type: "uint8" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "balanceOf",
    inputs: [{ type: "address" }],
    outputs: [{ type: "uint256" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "allowance",
    inputs: [{ type: "address" }, { type: "address" }],
    outputs: [{ type: "uint256" }],
    stateMutability: "view",
  },
] as const;

export interface HubChainConfigRow {
  chainKey: number;
  chainName: string;
  vault: `0x${string}`;
  outbox: `0x${string}`;
  enabled: boolean;
  /** attestToken.allowance(hub, outbox) — publishMessage pulls coreFee from this when non-zero. */
  outboxAttestAllowance: bigint;
}

export interface HubInspectData {
  address: `0x${string}`;
  owner: `0x${string}`;
  paused: boolean;
  proofVerifier: `0x${string}`;
  attestToken: {
    address: `0x${string}`;
    symbol: string;
    decimals: number;
    hubBalance: bigint;
  };
  chains: HubChainConfigRow[];
}

export function useHubInspect() {
  const query = useQuery({
    queryKey: ["inspect-hub", BRIDGE_HUB_ADDRESS],
    queryFn: async (): Promise<HubInspectData | null> => {
      if (!creditcoinPublicClient || !BRIDGE_HUB_ADDRESS) return null;
      const client = creditcoinPublicClient;
      const hub = BRIDGE_HUB_ADDRESS;

      const [owner, paused, proofVerifier, attestTokenAddress] =
        await Promise.all([
          client.readContract({
            address: hub,
            abi: BridgeHubAbi,
            functionName: "owner",
          }),
          client.readContract({
            address: hub,
            abi: BridgeHubAbi,
            functionName: "paused",
          }),
          client.readContract({
            address: hub,
            abi: BridgeHubAbi,
            functionName: "proofVerifier",
          }),
          client.readContract({
            address: hub,
            abi: BridgeHubAbi,
            functionName: "attestToken",
          }),
        ]);

      const [symbol, decimals, hubBalance] = await Promise.all([
        client.readContract({
          address: attestTokenAddress,
          abi: ERC20_ABI,
          functionName: "symbol",
        }),
        client.readContract({
          address: attestTokenAddress,
          abi: ERC20_ABI,
          functionName: "decimals",
        }),
        client.readContract({
          address: attestTokenAddress,
          abi: ERC20_ABI,
          functionName: "balanceOf",
          args: [hub],
        }),
      ]);

      const chains = await Promise.all(
        SPOKE_CHAINS.map(async (spoke): Promise<HubChainConfigRow> => {
          const [vault, outbox, enabled] = await client.readContract({
            address: hub,
            abi: BridgeHubAbi,
            functionName: "chainConfigs",
            args: [spoke.chainKey],
          });
          const outboxAttestAllowance =
            outbox && outbox !== "0x0000000000000000000000000000000000000000"
              ? await client.readContract({
                  address: attestTokenAddress,
                  abi: ERC20_ABI,
                  functionName: "allowance",
                  args: [hub, outbox],
                })
              : 0n;
          return {
            chainKey: spoke.chainKey,
            chainName: spoke.chain.name,
            vault,
            outbox,
            enabled,
            outboxAttestAllowance,
          };
        }),
      );

      return {
        address: hub,
        owner,
        paused,
        proofVerifier,
        attestToken: {
          address: attestTokenAddress,
          symbol,
          decimals,
          hubBalance,
        },
        chains,
      };
    },
    enabled: Boolean(creditcoinPublicClient && BRIDGE_HUB_ADDRESS),
    refetchInterval: 30_000,
  });

  return {
    available: Boolean(creditcoinPublicClient && BRIDGE_HUB_ADDRESS),
    data: query.data ?? undefined,
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  };
}

export interface VaultInspectData {
  address: `0x${string}`;
  owner: `0x${string}`;
  bridgeHub: `0x${string}`;
  bridgeHubMatchesConfigured: boolean | undefined;
  depositNonce: bigint;
  nativeBalance: bigint;
  /** undefined when this spoke has no configured router address to check against. */
  trustsConfiguredRouter: boolean | undefined;
}

export function useVaultInspect(spoke: SpokeChain | undefined) {
  const config = useConfig();

  const query = useQuery({
    queryKey: ["inspect-vault", spoke?.chainKey, spoke?.bridgeVaultAddress],
    queryFn: async (): Promise<VaultInspectData | null> => {
      if (!spoke) return null;
      const client = getPublicClient(config, { chainId: spoke.chain.id });
      if (!client) return null;

      const [owner, bridgeHub, depositNonce, nativeBalance] = await Promise.all(
        [
          client.readContract({
            address: spoke.bridgeVaultAddress,
            abi: BridgeVaultAbi,
            functionName: "owner",
          }),
          client.readContract({
            address: spoke.bridgeVaultAddress,
            abi: BridgeVaultAbi,
            functionName: "bridgeHub",
          }),
          client.readContract({
            address: spoke.bridgeVaultAddress,
            abi: BridgeVaultAbi,
            functionName: "depositNonce",
          }),
          client.getBalance({ address: spoke.bridgeVaultAddress }),
        ],
      );

      const trustsConfiguredRouter = spoke.dispatcherRouterAddress
        ? await client.readContract({
            address: spoke.bridgeVaultAddress,
            abi: BridgeVaultAbi,
            functionName: "isTrustedInbox",
            args: [spoke.dispatcherRouterAddress],
          })
        : undefined;

      return {
        address: spoke.bridgeVaultAddress,
        owner,
        bridgeHub,
        bridgeHubMatchesConfigured: BRIDGE_HUB_ADDRESS
          ? bridgeHub.toLowerCase() === BRIDGE_HUB_ADDRESS.toLowerCase()
          : undefined,
        depositNonce,
        nativeBalance,
        trustsConfiguredRouter,
      };
    },
    enabled: Boolean(spoke),
    refetchInterval: 30_000,
  });

  return {
    data: query.data ?? undefined,
    isLoading: query.isLoading,
    error: query.error ?? undefined,
  };
}
