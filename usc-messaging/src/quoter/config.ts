/**
 * Quoter configuration.
 * Uses CLI args first, then env vars, then defaults.
 *
 * Start: node dist/quoter/server.js --payee-address 0x... --rpc-url http://...
 */

import dotenv from "dotenv";

import { DEFAULT_DESTINATION_RPC_URL, DEFAULT_QUOTER_PORT } from "../consts.js";
import { isValidContractAddress, isValidPrivateKey } from "../utils.js";

const DEFAULT_SIGNER_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const DEFAULT_PAYEE = "0x0000000000000000000000000000000000000001";
const DEFAULT_PAYMENT_TOKEN = "0x0000000000000000000000000000000000000000";
const DEFAULT_EXPIRY_SECONDS = "3600";
const DEFAULT_RELAY_GAS_LIMIT = "300000";
const DEFAULT_ACK_GAS_LIMIT = "500000";
const DEFAULT_GAS_BUFFER = "135";

export interface QuoterConfig {
  /** Port for the HTTP server */
  port: number;
  /** Private key for signing quotes (hex string with 0x prefix) */
  signerPrivateKey: string;
  /** Relayer pool / payee address to receive payments */
  payeeAddress: string;
  /** Payment token address (0x0 for native) */
  paymentToken: string;
  /** Quote validity in seconds */
  quoteExpirySeconds: number;
  /** Default relay gas limit (deliverMessage + target execution) */
  defaultRelayGasLimit: bigint;
  /** Acknowledgment gas (proof submission on Creditcoin L1) */
  ackGasLimit: bigint;
  /** Gas buffer multiplier (e.g. 135 = 35% buffer) */
  gasBufferMultiplier: number;
  /** RPC URL for destination chain (for eth_gasPrice, eth_chainId) */
  destinationChainRpcUrl?: string;
  /** Chain ID derived from destinationChainRpcUrl (set at startup when URL provided) */
  destinationChainId?: number;
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

export async function loadQuoterConfig(): Promise<QuoterConfig> {
  dotenv.config({ override: true });

  const payeeAddress =
    parseArg("--payee-address", "-p") ??
    process.env.QUOTER_PAYEE_ADDRESS ??
    DEFAULT_PAYEE;

  const paymentToken =
    parseArg("--payment-token", "-t") ??
    process.env.QUOTER_PAYMENT_TOKEN ??
    DEFAULT_PAYMENT_TOKEN;

  const destinationChainRpcUrl =
    parseArg("--rpc-url", "-r") ??
    process.env.QUOTER_DESTINATION_RPC_URL ??
    process.env.QUOTER_RPC_URL ??
    DEFAULT_DESTINATION_RPC_URL;

  const key =
    parseArg("--private-key", "-k") ??
    process.env.QUOTER_SIGNER_PRIVATE_KEY ??
    DEFAULT_SIGNER_KEY;

  const port = parseInt(
    parseArg("--port") ?? process.env.QUOTER_PORT ?? DEFAULT_QUOTER_PORT,
    10,
  );

  if (!isValidContractAddress(payeeAddress)) {
    throw new Error(
      `Invalid payee address: ${payeeAddress}. Pass --payee-address 0x<40 hex chars> or set QUOTER_PAYEE_ADDRESS.`,
    );
  }
  if (!isValidContractAddress(paymentToken)) {
    throw new Error(
      `Invalid payment token: ${paymentToken}. Pass --payment-token 0x<40 hex chars> or set QUOTER_PAYMENT_TOKEN.`,
    );
  }
  if (!isValidPrivateKey(key)) {
    throw new Error(
      "Invalid or missing private key. Pass --private-key 0x<64 hex chars> or set QUOTER_SIGNER_PRIVATE_KEY.",
    );
  }

  let destinationChainId: number | undefined;
  if (destinationChainRpcUrl) {
    try {
      const { ethers } = await import("ethers");
      const provider = new ethers.JsonRpcProvider(destinationChainRpcUrl);
      const network = await provider.getNetwork();
      destinationChainId = Number(network.chainId);
    } catch (err) {
      throw new Error(
        `Failed to fetch chain ID from RPC ${destinationChainRpcUrl}: ${err instanceof Error ? err.message : err}`,
      );
    }
  }

  return {
    port,
    signerPrivateKey: key.startsWith("0x") ? key : `0x${key}`,
    payeeAddress,
    paymentToken,
    quoteExpirySeconds: parseInt(
      process.env.QUOTER_EXPIRY_SECONDS ?? DEFAULT_EXPIRY_SECONDS,
      10,
    ),
    defaultRelayGasLimit: BigInt(
      process.env.QUOTER_RELAY_GAS_LIMIT ?? DEFAULT_RELAY_GAS_LIMIT,
    ),
    ackGasLimit: BigInt(
      process.env.QUOTER_ACK_GAS_LIMIT ?? DEFAULT_ACK_GAS_LIMIT,
    ),
    gasBufferMultiplier: parseInt(
      process.env.QUOTER_GAS_BUFFER ?? DEFAULT_GAS_BUFFER,
      10,
    ),
    destinationChainRpcUrl,
    destinationChainId,
  };
}
