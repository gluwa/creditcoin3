/**
 * Exchange rate module for the quoter.
 * Phase 1: config-based (dummy) rates.
 * Phase 2+: integrate Chainlink, API, etc.
 */

import type { ExchangeRates } from "./types.js";

/** Default rates: native currency units per 1e18 attest coin.
 * E.g. 1e18 means 1 ETH = 1 attest coin (1:1 for dev) */
const DEFAULT_RATES: ExchangeRates = {
  1: "1000000000000000000", // Ethereum
  31337: "1000000000000000000", // Anvil1
  31338: "1000000000000000000", // Anvil2
  31339: "1000000000000000000", // Anvil3
  11155111: "1000000000000000000", // Sepolia
  80002: "1000000000000000000", // Polygon amoy
};

let customRates: ExchangeRates | null = null;

/**
 * Set custom exchange rates (e.g. from config file).
 * Call before getExchangeRate.
 */
export function setExchangeRates(rates: ExchangeRates): void {
  customRates = rates;
}

/**
 * Get exchange rate: native currency units per 1e18 payment token.
 * Returns wei amount of native currency that equals 1e18 payment token units.
 */
export function getExchangeRate(chainId: number): bigint {
  const rates = customRates ?? DEFAULT_RATES;
  const rate = rates[chainId];
  if (!rate) {
    throw new Error(`No exchange rate configured for chain ID ${chainId}`);
  }
  return BigInt(rate);
}
