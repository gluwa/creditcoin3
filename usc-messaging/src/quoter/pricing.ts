// Phase-1 static pricing: convert a destination gas budget into an ATTEST fee.
//
//   relayPrice(ATTEST wei) = gasLimit × gasPrice(dst native wei)
//                            × nativeUsd / attestUsd × (1 + buffer)
//
// USD prices are held as 1e10 fixed-point bigints so the whole computation stays in integer math
// (the two USD scales cancel, and dst-native wei and ATTEST wei are both 18-decimal).
import { ethers } from "ethers";
import type { ChainConfig, QuoterConfig } from "./config.js";

const USD_SCALE = 10; // decimals for the fixed-point USD prices

export interface PricedFee {
  relayPrice: bigint;
  acknowledgmentPrice: bigint;
  coreFee: bigint;
  gasPriceWei: bigint;
}

/** Live destination gas price when an RPC is configured, else the static fallback. */
async function resolveGasPrice(chain: ChainConfig): Promise<bigint> {
  const fallback = ethers.parseUnits(chain.gasPriceGwei, "gwei");
  if (!chain.rpcUrl) return fallback;
  try {
    const provider = new ethers.JsonRpcProvider(chain.rpcUrl);
    const fee = await provider.getFeeData();
    return fee.gasPrice ?? fee.maxFeePerGas ?? fallback;
  } catch {
    return fallback; // never let a flaky RPC block a quote — fall back to the static price
  }
}

/** ATTEST wei for a given dst-native-wei cost, at the static ATTEST/USD and native/USD prices. */
function nativeToAttest(
  nativeWei: bigint,
  chain: ChainConfig,
  cfg: QuoterConfig,
): bigint {
  const nativeUsd = ethers.parseUnits(chain.nativeUsd, USD_SCALE);
  const attestUsd = ethers.parseUnits(cfg.attestUsd, USD_SCALE);
  if (attestUsd === 0n) throw new Error("attestUsd must be > 0");
  return (nativeWei * nativeUsd) / attestUsd;
}

export async function priceFee(
  chain: ChainConfig,
  cfg: QuoterConfig,
  gasLimit: bigint,
  requiresAck: boolean,
): Promise<PricedFee> {
  const gasPriceWei = await resolveGasPrice(chain);
  const buffer = (x: bigint): bigint =>
    (x * (10_000n + cfg.priceBufferBps)) / 10_000n;

  const relayPrice = buffer(nativeToAttest(gasLimit * gasPriceWei, chain, cfg));
  const acknowledgmentPrice = requiresAck
    ? buffer(nativeToAttest(chain.ackGas * gasPriceWei, chain, cfg))
    : 0n;

  return {
    relayPrice,
    acknowledgmentPrice,
    coreFee: chain.coreFeeWei,
    gasPriceWei,
  };
}
