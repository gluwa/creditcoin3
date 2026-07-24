// Offline self-test: proves the signed quote is byte-compatible with RelayerContract._validateQuote
// (no chain needed). Run with: npm run quoter:selftest
import { ethers } from "ethers";
import {
  quoteDigest,
  signQuote,
  type QuoteFields,
  type QuoteDomain,
} from "./quote.js";

const QUOTE_TUPLE =
  "tuple(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bool requiresAck,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bytes signature)";

async function main(): Promise<void> {
  const wallet = ethers.Wallet.createRandom();
  const fields: QuoteFields = {
    coreFee: 1_000000000000000000n,
    relayPrice: 42_000000000000000000n,
    acknowledgmentPrice: 7_000000000000000000n,
    gasLimit: 300_000n,
    destinationChain: 11155111,
    requiresAck: true,
    payloadHash: ethers.keccak256(ethers.toUtf8Bytes("hello world")),
    targetContract: "0x1111111111111111111111111111111111111111",
    expectedCompletion: 1_800_000_000n,
    expiry: 1_900_000_000n,
  };
  const domain: QuoteDomain = {
    sourceChainId: 42n,
    verifyingContract: "0x2222222222222222222222222222222222222222",
  };

  const signed = await signQuote(
    wallet as unknown as ethers.Wallet,
    fields,
    domain,
  );

  // 1. Recovery must match the Quoter EOA — this is exactly what the contract does:
  //    MessageHashUtils.toEthSignedMessageHash(digest).recover(signature).
  const digest = quoteDigest(fields, domain);
  const recovered = ethers.verifyMessage(
    ethers.getBytes(digest),
    signed.signature,
  );
  if (recovered.toLowerCase() !== wallet.address.toLowerCase())
    throw new Error(`recovery mismatch: ${recovered} != ${wallet.address}`);

  // 2. The digest must be domain-bound: a different verifyingContract → different digest.
  const otherDigest = quoteDigest(fields, {
    ...domain,
    verifyingContract: "0x3333333333333333333333333333333333333333",
  });
  if (otherDigest === digest)
    throw new Error("digest is not bound to verifyingContract");

  // 3. signedQuote must abi-decode back into the same struct the contract reads.
  const [decoded] = ethers.AbiCoder.defaultAbiCoder().decode(
    [QUOTE_TUPLE],
    signed.signedQuote,
  );
  if (BigInt(decoded.relayPrice) !== fields.relayPrice)
    throw new Error("relayPrice did not round-trip");
  if (Number(decoded.destinationChain) !== fields.destinationChain)
    throw new Error("destinationChain did not round-trip");
  if (decoded.requiresAck !== fields.requiresAck)
    throw new Error("requiresAck did not round-trip");
  if (decoded.payloadHash !== fields.payloadHash)
    throw new Error("payloadHash did not round-trip");
  if (decoded.signature !== signed.signature)
    throw new Error("signature did not round-trip");

  console.log("✅ selftest passed");
  console.log("   quoter   :", wallet.address);
  console.log("   digest   :", digest);
  console.log("   recovered:", recovered);
  console.log("   signedQuote bytes:", ethers.dataLength(signed.signedQuote));
}

main().catch((err) => {
  console.error("❌ selftest failed:", err);
  process.exit(1);
});
