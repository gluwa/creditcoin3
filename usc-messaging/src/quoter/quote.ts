/**
 * Quote computation and signing.
 */

import { ethers } from "ethers";
import { getExchangeRate } from "./exchange-rates.js";
import type { QuoteData, QuoteRequest, SignedQuote } from "./types.js";
import type { QuoterConfig } from "./config.js";

/** EIP-712 style hash for quote - keccak256 of packed values for simplicity.
 * RelayerContract will need to use the same encoding. */
function encodeQuoteForSigning(data: QuoteData): string {
  return ethers.solidityPackedKeccak256(
    ["uint256", "uint256", "address", "address", "uint256"],
    [
      data.relayPrice,
      data.acknowledgmentPrice,
      data.payeeAddress,
      data.paymentToken,
      data.expiry,
    ]
  );
}

/**
 * Fetch gas price from destination chain RPC, or use a fallback.
 */
async function getGasPrice(config: QuoterConfig, chainId: number): Promise<bigint> {
  if (config.destinationChainRpcUrl) {
    try {
      const provider = new ethers.JsonRpcProvider(config.destinationChainRpcUrl);
      const feeData = await provider.getFeeData();
      const gasPrice = feeData.gasPrice ?? 0n;
      if (gasPrice > 0n) return gasPrice;
    } catch (err) {
      console.warn("Failed to fetch gas price from RPC, using fallback:", err);
    }
  }
  // Fallback: 30 gwei for testnets, 50 for mainnet
  return chainId === 1 ? 50n * 10n ** 9n : 30n * 10n ** 9n;
}

/**
 * Compute and sign a quote for the given request.
 */
export async function createQuote(
  request: QuoteRequest,
  config: QuoterConfig
): Promise<SignedQuote> {
  const gasPrice = await getGasPrice(config, request.destinationChainId);
  const exchangeRate = getExchangeRate(request.destinationChainId);

  const gasLimit = request.gasLimit ?? config.defaultRelayGasLimit;
  const bufferedGas = (gasLimit * BigInt(config.gasBufferMultiplier)) / 100n;
  const relayCostNative = bufferedGas * gasPrice;
  const relayPrice = (relayCostNative * 10n ** 18n) / exchangeRate;

  const ackCostNative = request.requiresAck
    ? (config.ackGasLimit * gasPrice * 10n ** 18n) / exchangeRate
    : 0n;

  const expiry = Math.floor(Date.now() / 1000) + config.quoteExpirySeconds;

  const quoteData: QuoteData = {
    relayPrice,
    acknowledgmentPrice: ackCostNative,
    payeeAddress: config.payeeAddress,
    paymentToken: config.paymentToken,
    expiry,
  };

  const hash = encodeQuoteForSigning(quoteData);
  const wallet = new ethers.Wallet(config.signerPrivateKey);
  // Sign the raw hash (no Ethereum signed message prefix) so RelayerContract ecrecover works
  const sig = wallet.signingKey.sign(ethers.getBytes(hash));
  const signature = sig.serialized;

  return {
    ...quoteData,
    signature,
  };
}
