// Read-only Creditcoin access for the progress screen's best-effort "claimed" step. Never a wagmi
// chain and never wallet-connected — users only ever hold a wallet on a spoke chain, matching the
// "invisible hub" design. Unset env vars degrade this to `undefined`, and callers (use-transfer-
// status.ts) must treat that as "skip this step" rather than an error.
import { createPublicClient, defineChain, http } from "viem";

const CHAIN_ID = Number(process.env.NEXT_PUBLIC_CREDITCOIN_CHAIN_ID ?? "42");
const RPC_URL = process.env.NEXT_PUBLIC_CREDITCOIN_RPC_URL;
export const BRIDGE_HUB_ADDRESS = process.env.NEXT_PUBLIC_BRIDGE_HUB_ADDRESS as
  `0x${string}` | undefined;

export const creditcoinPublicClient =
  RPC_URL && BRIDGE_HUB_ADDRESS
    ? createPublicClient({
        chain: defineChain({
          id: CHAIN_ID,
          name: "Creditcoin",
          nativeCurrency: { name: "CTC", symbol: "CTC", decimals: 18 },
          rpcUrls: { default: { http: [RPC_URL] } },
        }),
        transport: http(RPC_URL),
      })
    : undefined;
