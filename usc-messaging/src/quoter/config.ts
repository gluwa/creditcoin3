/**
 * Quoter configuration.
 * Uses CLI args first, then env vars, then defaults.
 */

export interface QuoterConfig {
  /** Port for the HTTP server */
  port: number;
  /** Private key for signing quotes (hex string, no 0x prefix or with) */
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

const DEFAULT_SIGNER_KEY =
  "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const DEFAULT_PAYEE = "0x0000000000000000000000000000000000000001";

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

function isValidAddress(s: string): boolean {
  return /^0x[a-fA-F0-9]{40}$/.test(s);
}

export async function loadQuoterConfig(): Promise<QuoterConfig> {
  const payeeArg = parseArg("--payee-address", "-p");
  const tokenArg = parseArg("--payment-token", "-t");
  const rpcUrlArg = parseArg("--rpc-url", "-r");

  const payeeAddress = payeeArg ?? process.env.QUOTER_PAYEE_ADDRESS ?? DEFAULT_PAYEE;
  const paymentToken =
    tokenArg ??
    process.env.QUOTER_PAYMENT_TOKEN ??
    "0x0000000000000000000000000000000000000000";
  const destinationChainRpcUrl =
    rpcUrlArg ?? process.env.QUOTER_DESTINATION_RPC_URL ?? process.env.QUOTER_RPC_URL;

  if (!isValidAddress(payeeAddress)) {
    throw new Error(`Invalid payeeAddress: ${payeeAddress}`);
  }
  if (!isValidAddress(paymentToken)) {
    throw new Error(`Invalid paymentToken: ${paymentToken}`);
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
        `Failed to fetch chain ID from RPC ${destinationChainRpcUrl}: ${err instanceof Error ? err.message : err}`
      );
    }
  }

  return {
    port: parseInt(process.env.QUOTER_PORT ?? "3300", 10),
    signerPrivateKey: process.env.QUOTER_SIGNER_PRIVATE_KEY ?? DEFAULT_SIGNER_KEY,
    payeeAddress,
    paymentToken,
    quoteExpirySeconds: parseInt(process.env.QUOTER_EXPIRY_SECONDS ?? "3600", 10),
    defaultRelayGasLimit: BigInt(process.env.QUOTER_RELAY_GAS_LIMIT ?? "300000"),
    ackGasLimit: BigInt(process.env.QUOTER_ACK_GAS_LIMIT ?? "500000"),
    gasBufferMultiplier: parseInt(process.env.QUOTER_GAS_BUFFER ?? "135", 10),
    destinationChainRpcUrl,
    destinationChainId,
  };
}
