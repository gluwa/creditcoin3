// Build and sign a RelayerTypes.Quote exactly as RelayerContract._validateQuote expects it.
//
// Digest (what the Quoter EOA signs) — EIP-712-style struct hash, domain-bound:
//   keccak256(abi.encode(
//     RELAYER_QUOTE_TYPEHASH,
//     coreFee, relayPrice, acknowledgmentPrice, gasLimit, destinationChain,
//     payloadHash, targetContract, expectedCompletion, expiry, payInNative,
//     sourceChainId, verifyingContract))          // sourceChainId = block.chainid, verifyingContract = RelayerContract
// then EIP-191 personal-sign (MessageHashUtils.toEthSignedMessageHash), which is what
// ethers `Wallet.signMessage(getBytes(digest))` produces.
//
// v3 preimage (usc-contracts #23/#28): the struct no longer carries a `requiresAck` flag — a
// nonzero acknowledgmentPrice IS the acknowledgment request (RelayerContract derives canAck from
// it). `payInNative` was added so relayPrice (+ tip) can be settled in destination-chain native
// coin instead of ATTEST; coreFee and acknowledgmentPrice are always ATTEST regardless.
//
// signedQuote (passed to RelayerContract) = abi.encode of the full Quote struct incl. signature.
import { ethers } from "ethers";

// Must match RelayerTypes.QUOTE_TYPEHASH byte-for-byte.
const RELAYER_QUOTE_TYPEHASH = ethers.id(
  "RelayerQuote(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,uint256 sourceChainId,address verifyingContract)",
);

// Digest field order MUST match RelayerContract._validateQuote's abi.encode.
const DIGEST_TYPES = [
  "bytes32", // RELAYER_QUOTE_TYPEHASH
  "uint256", // coreFee
  "uint256", // relayPrice
  "uint256", // acknowledgmentPrice
  "uint256", // gasLimit
  "uint32", // destinationChain
  "bytes32", // payloadHash
  "address", // targetContract
  "uint256", // expectedCompletion
  "uint256", // expiry
  "bool", // payInNative
  "uint256", // sourceChainId
  "address", // verifyingContract
];

// Field order MUST match RelayerTypes.Quote (payInNative added before signature).
const QUOTE_TUPLE =
  "tuple(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,bytes signature)";

export interface QuoteFields {
  coreFee: bigint;
  relayPrice: bigint;
  acknowledgmentPrice: bigint;
  gasLimit: bigint;
  destinationChain: number;
  payloadHash: string;
  targetContract: string;
  expectedCompletion: bigint;
  expiry: bigint;
  payInNative: boolean;
}

/** The domain the quote signature is bound to — must match the deployed RelayerContract. */
export interface QuoteDomain {
  /** block.chainid of the chain the RelayerContract lives on (Creditcoin EVM). */
  sourceChainId: bigint;
  /** The RelayerContract address that will verify the quote. */
  verifyingContract: string;
}

export interface SignedQuote extends QuoteFields {
  signature: string;
  /** ABI-encoded Quote struct — pass this to RelayerContract as `signedQuote`. */
  signedQuote: string;
}

export function quoteDigest(q: QuoteFields, d: QuoteDomain): string {
  const encoded = ethers.AbiCoder.defaultAbiCoder().encode(DIGEST_TYPES, [
    RELAYER_QUOTE_TYPEHASH,
    q.coreFee,
    q.relayPrice,
    q.acknowledgmentPrice,
    q.gasLimit,
    q.destinationChain,
    q.payloadHash,
    q.targetContract,
    q.expectedCompletion,
    q.expiry,
    q.payInNative,
    d.sourceChainId,
    ethers.getAddress(d.verifyingContract),
  ]);
  return ethers.keccak256(encoded);
}

export async function signQuote(
  wallet: ethers.Wallet,
  q: QuoteFields,
  d: QuoteDomain,
): Promise<SignedQuote> {
  const digest = quoteDigest(q, d);
  // signMessage over the raw 32 bytes applies the EIP-191 prefix, matching the contract's
  // MessageHashUtils.toEthSignedMessageHash(digest).recover(signature).
  const signature = await wallet.signMessage(ethers.getBytes(digest));

  const signedQuote = ethers.AbiCoder.defaultAbiCoder().encode(
    [QUOTE_TUPLE],
    [
      [
        q.coreFee,
        q.relayPrice,
        q.acknowledgmentPrice,
        q.gasLimit,
        q.destinationChain,
        q.payloadHash,
        q.targetContract,
        q.expectedCompletion,
        q.expiry,
        q.payInNative,
        signature,
      ],
    ],
  );

  return { ...q, signature, signedQuote };
}
