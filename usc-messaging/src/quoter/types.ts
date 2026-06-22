/**
 * Quote types for the USC Write-Ability Layer.
 * Aligns with usc-write-ability-research documents/07-quotation-system.md
 */

/** Chain ID (EVM chainId) for destination chain */
export type ChainId = number;

/** Quote request parameters */
export interface QuoteRequest {
  /** Message ID (bytes32 hex) the quote is being issued for */
  messageId: string;
  /** Destination chain EVM chain ID (e.g. 31337 for Anvil, 1 for Ethereum) */
  destinationChainId: ChainId;
  /** Whether the message requires acknowledgment */
  requiresAck: boolean;
  /** Optional: custom gas limit for delivery (default: estimated) */
  gasLimit?: bigint;
}

/** Raw quote data before signing (matches Solidity struct layout for hashing) */
export interface QuoteData {
  relayPrice: bigint;
  acknowledgmentPrice: bigint;
  payeeAddress: string;
  paymentToken: string;
  expiry: number;
}

/** Signed quote returned to the client */
export interface SignedQuote extends QuoteData {
  /** Message ID (bytes32 hex) this quote was issued for */
  messageId: string;
  /** ECDSA signature over keccak256(messageId, relayPrice, acknowledgmentPrice, payeeAddress, paymentToken, expiry) */
  signature: string;
}

/** Exchange rate config: native currency units per 1 attest coin (or payment token unit) */
export interface ExchangeRates {
  /** Chain ID -> rate (e.g. 1e18 wei per 1e18 attest coin) */
  [chainId: number]: string;
}
