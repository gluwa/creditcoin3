// Build and sign a RelayerTypes.Quote exactly as RelayerContract._validateQuote expects it.
//
// Digest (what the Quoter EOA signs) — EIP-712-style struct hash, domain-bound:
//   keccak256(abi.encode(
//     RELAYER_QUOTE_TYPEHASH,
//     coreFee, relayPrice, acknowledgmentPrice, gasLimit, destinationChain, requiresAck,
//     payloadHash, targetContract, expectedCompletion, expiry,
//     sourceChainId, verifyingContract))          // sourceChainId = block.chainid, verifyingContract = RelayerContract
// then EIP-191 personal-sign (MessageHashUtils.toEthSignedMessageHash), which is what
// ethers `Wallet.signMessage(getBytes(digest))` produces.
//
// signedQuote (passed to RelayerContract) = abi.encode of the full Quote struct incl. signature.
import { ethers } from "ethers";

// Must match RelayerContract._QUOTE_TYPEHASH byte-for-byte.
const RELAYER_QUOTE_TYPEHASH = ethers.id(
  "RelayerQuote(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bool requiresAck,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,uint256 sourceChainId,address verifyingContract)",
);

// Digest field order MUST match RelayerContract._validateQuote's abi.encode.
const DIGEST_TYPES = [
  "bytes32", // RELAYER_QUOTE_TYPEHASH
  "uint256", // coreFee
  "uint256", // relayPrice
  "uint256", // acknowledgmentPrice
  "uint256", // gasLimit
  "uint32", // destinationChain
  "bool", // requiresAck
  "bytes32", // payloadHash
  "address", // targetContract
  "uint256", // expectedCompletion
  "uint256", // expiry
  "uint256", // sourceChainId
  "address", // verifyingContract
];

// Field order MUST match RelayerTypes.Quote (requiresAck added after destinationChain).
const QUOTE_TUPLE =
  "tuple(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bool requiresAck,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bytes signature)";

export interface QuoteFields {
  coreFee: bigint;
  relayPrice: bigint;
  acknowledgmentPrice: bigint;
  gasLimit: bigint;
  destinationChain: number;
  requiresAck: boolean;
  payloadHash: string;
  targetContract: string;
  expectedCompletion: bigint;
  expiry: bigint;
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
    q.requiresAck,
    q.payloadHash,
    q.targetContract,
    q.expectedCompletion,
    q.expiry,
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
        q.requiresAck,
        q.payloadHash,
        q.targetContract,
        q.expectedCompletion,
        q.expiry,
        signature,
      ],
    ],
  );

  return { ...q, signature, signedQuote };
}
