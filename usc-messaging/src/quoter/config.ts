// Configuration for the light-weight (Phase 1) USC write-ability quoter.
//
// Phase 1 keeps pricing deliberately simple: ATTEST is priced at a *static* USD value (the coin is
// not live yet), destination gas is either read live from an RPC or taken from a static fallback,
// and the destination native-token USD price is static per chain. This is enough to produce
// sane, signed quotes the RelayerContract accepts, without an external price oracle. The on-chain
// USCRelayingQuoter / oracle-push path replaces these statics later.
import "dotenv/config";

export interface ChainConfig {
  /** USC destination chain key — the value carried in `Quote.destinationChain` and used on-chain
   *  for `getCoreFee(uint16(destinationChain))`. This is the USC key (e.g. 2 = local anvil,
   *  3 = Sepolia), NOT the EVM chain id, so quotes match the on-chain coreFee table. */
  chainKey: number;
  /** Static USD price of the destination native token, e.g. "3000" for ETH. */
  nativeUsd: string;
  /** Fallback destination gas price in gwei, used when no rpcUrl is set or the RPC read fails. */
  gasPriceGwei: string;
  /** Optional destination RPC; when set the quoter reads the live gas price from it. */
  rpcUrl?: string;
  /** Core (protocol) fee for this destination, in ATTEST wei. Informational in the quote — the
   *  Outbox reads the authoritative value live from USCRelayingQuoter.getCoreFee at publish. */
  coreFeeWei: bigint;
  /** Gas units assumed for the acknowledgment relay when requiresAck = true. */
  ackGas: bigint;
}

export interface QuoterConfig {
  port: number;
  /** Private key of the authorized Quoter EOA (must be whitelisted in USCRelayingQuoter). */
  privateKey: string;
  /** Static ATTEST/USD price (Phase 1: coin not live). Default $0.10. */
  attestUsd: string;
  /** Quote validity window in seconds. */
  quoteTtlSecs: number;
  /** Estimated delivery time in seconds, used for Quote.expectedCompletion. */
  estimatedDeliverySecs: number;
  /** Overhead buffer in basis points added on top of the raw gas cost (absorbs gas drift). */
  priceBufferBps: bigint;
  /** block.chainid of the chain the RelayerContract lives on (Creditcoin EVM). Quote domain. */
  sourceChainId: bigint;
  /** RelayerContract address the quote signature binds to (quote domain). Must match the deploy. */
  relayerContract: string;
  chains: Map<number, ChainConfig>;
}

function req(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing required env ${name}`);
  return v;
}

// Static per-chain defaults. Override the whole set with QUOTER_CHAINS (JSON array of ChainConfig,
// coreFee expressed in whole ATTEST as `coreFeeAttest`).
const DEFAULT_CHAINS: ChainConfig[] = [
  {
    chainKey: 2, // local anvil ("Anvil1") — USC chain key 2
    nativeUsd: "3000",
    gasPriceGwei: "1",
    rpcUrl: process.env.DESTINATION_CHAIN_RPC_URL,
    coreFeeWei: 1_000000000000000000n, // 1 ATTEST
    ackGas: 500_000n,
  },
  {
    chainKey: 3, // Sepolia — USC chain key 3
    nativeUsd: "3000",
    gasPriceGwei: "5",
    rpcUrl: process.env.SEPOLIA_RPC_URL,
    coreFeeWei: 1_000000000000000000n, // 1 ATTEST
    ackGas: 500_000n,
  },
];

function loadChains(): Map<number, ChainConfig> {
  const raw = process.env.QUOTER_CHAINS;
  const list: ChainConfig[] = raw
    ? (JSON.parse(raw) as Array<Record<string, unknown>>).map((c) => ({
        // Accept `chainKey` (preferred) or legacy `chainId` for the USC destination chain key.
        chainKey: Number(c.chainKey ?? c.chainId),
        nativeUsd: String(c.nativeUsd),
        gasPriceGwei: String(c.gasPriceGwei ?? "1"),
        rpcUrl: c.rpcUrl ? String(c.rpcUrl) : undefined,
        coreFeeWei:
          BigInt(Math.round(Number(c.coreFeeAttest ?? "1") * 1e9)) *
          1_000000000n, // ATTEST → wei via 1e9 * 1e9 to keep 9 decimals of precision
        ackGas: BigInt(Number(c.ackGas ?? 500_000)),
      }))
    : DEFAULT_CHAINS;
  return new Map(list.map((c) => [c.chainKey, c]));
}

export function loadConfig(): QuoterConfig {
  return {
    port: Number(process.env.QUOTER_PORT ?? "3010"),
    privateKey: req("QUOTER_PRIVATE_KEY"),
    attestUsd: process.env.QUOTER_ATTEST_USD ?? "0.10",
    quoteTtlSecs: Number(process.env.QUOTER_QUOTE_TTL_SECS ?? "300"),
    estimatedDeliverySecs: Number(
      process.env.QUOTER_EST_DELIVERY_SECS ?? "120",
    ),
    priceBufferBps: BigInt(process.env.QUOTER_PRICE_BUFFER_BPS ?? "1000"),
    sourceChainId: BigInt(process.env.QUOTER_SOURCE_CHAIN_ID || "42"),
    // `||` (not `??`): an empty string from a copied .env.example must fall back to the zero
    // address, else quote signing calls ethers.getAddress("") and throws a 500 on /quote.
    relayerContract:
      process.env.QUOTER_RELAYER_CONTRACT ||
      "0x0000000000000000000000000000000000000000",
    chains: loadChains(),
  };
}
