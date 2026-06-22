#!/usr/bin/env npx tsx
/**
 * Requests a quote from the quoter service and forwards it to the
 * SimpleRelayer contract for validation and fee collection.
 *
 * Prerequisites:
 *   - Quoter service running (npm run dev:quoter)
 *   - SimpleRelayer deployed (address provided via env or deployments.json)
 *   - Anvil (or target chain) running
 *
 * Usage:
 *   tsx scripts/request-and-relay-quote.ts [options]
**/

import "dotenv/config";
import { ethers } from "ethers";

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

// ── CLI arg helpers ──────────────────────────────────────────────────────────

function cliArg(long: string, short: string): string | undefined {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === `--${long}` || args[i] === `-${short}`) {
      return args[i + 1];
    }
  }
  return undefined;
}

// ── Configuration ────────────────────────────────────────────────────────────

const DEFAULT_MESSAGE_ID =
  "0x0000000000000000000000000000000000000000000000000000000000000001";

const messageId =
  cliArg("message-id", "m") ?? DEFAULT_MESSAGE_ID;
const quoterUrl = cliArg("quoter-url", "q") ??
  `http://127.0.0.1:${process.env.QUOTER_PORT ?? "3300"}`;
const sourceRpcUrl = requireEnv("CREDITCOIN_RPC_URL");
const quoterSourceChainKey = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");
const destinationChainId =
  cliArg("chain-id", "d") ?? requireEnv("DESTINATION_CHAIN_ID");
const relayerContractAddr = requireEnv("RELAYER_CONTRACT_ADDR");

// SimpleRelayer ABI — only the function we need
const SIMPLE_RELAYER_ABI = [
  "function validateAndCollectFee(tuple(bytes32 messageId, uint256 relayPrice, uint256 acknowledgmentPrice, address payeeAddress, address paymentToken, uint256 expiry, bytes signature) quote) payable",
  "function registerQuoter(address quoter) external",
  "function registeredQuoters(address) view returns (bool)",
  "event MessagePaid(bytes32 indexed messageId)",
  "event FeeCollected(address indexed from, uint256 amount)",
];

// ── Helpers ──────────────────────────────────────────────────────────────────

interface QuoteResponse {
  messageId: string;
  relayPrice: string;
  acknowledgmentPrice: string;
  payeeAddress: string;
  paymentToken: string;
  expiry: number;
  signature: string;
}

async function fetchQuote(baseUrl: string): Promise<QuoteResponse> {
  const params = new URLSearchParams({ messageId });
  params.set("destinationChainId", destinationChainId);

  const url = `${baseUrl}/quote?${params}`;
  console.log(`Requesting quote: ${url}`);

  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Quoter returned ${res.status}: ${body}`);
  }
  return (await res.json()) as QuoteResponse;
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main() {
  // 1. Request a quote from the quoter service
  const quote = await fetchQuote(quoterUrl);
  console.log("\nReceived quote:");
  console.log(`  messageId:            ${quote.messageId}`);
  console.log(`  relayPrice:           ${quote.relayPrice}`);
  console.log(`  acknowledgmentPrice:  ${quote.acknowledgmentPrice}`);
  console.log(`  payeeAddress:         ${quote.payeeAddress}`);
  console.log(`  paymentToken:         ${quote.paymentToken}`);
  console.log(`  expiry:               ${quote.expiry}`);
  console.log(`  signature:            ${quote.signature.slice(0, 20)}...`);

  // 2. Connect to relayer contract on source chain
  const provider = new ethers.JsonRpcProvider(sourceRpcUrl);
  const wallet = new ethers.Wallet(quoterSourceChainKey, provider);
  const relayer = new ethers.Contract(
    relayerContractAddr,
    SIMPLE_RELAYER_ABI,
    wallet,
  );

  // 3. Check if the quoter is registered
  const quoterSigner = ethers.computeAddress(quoterSourceChainKey);
  const isRegistered = await relayer.registeredQuoters(quoterSigner);
  if (!isRegistered) {
    console.log(
      `\nQuoter ${quoterSigner} not registered — registering now...`,
    );
    const regTx = await relayer.registerQuoter(quoterSigner);
    await regTx.wait();
    console.log(`Registered quoter (tx: ${regTx.hash})`);
  } else {
    console.log(`\nQuoter ${quoterSigner} is registered.`);
  }

  // 4. Submit the quote to SimpleRelayer.validateAndCollectFee
  const totalPrice =
    BigInt(quote.relayPrice) + BigInt(quote.acknowledgmentPrice);
  console.log(
    `\nSubmitting quote to relayer (value: ${ethers.formatEther(totalPrice)} ETH)...`,
  );

  const tx = await relayer.validateAndCollectFee(
    {
      messageId: quote.messageId,
      relayPrice: quote.relayPrice,
      acknowledgmentPrice: quote.acknowledgmentPrice,
      payeeAddress: quote.payeeAddress,
      paymentToken: quote.paymentToken,
      expiry: quote.expiry,
      signature: quote.signature,
    },
    { value: totalPrice },
  );

  const receipt = await tx.wait();
  console.log(`\nTransaction confirmed!`);
  console.log(`  tx hash:    ${receipt.hash}`);
  console.log(`  block:      ${receipt.blockNumber}`);
  console.log(`  gas used:   ${receipt.gasUsed}`);

  // 5. Check for emitted events
  for (const log of receipt.logs) {
    try {
      const parsed = relayer.interface.parseLog(log);
      if (parsed) {
        console.log(`  event:      ${parsed.name}(${parsed.args.join(", ")})`);
      }
    } catch {
      // skip logs from other contracts
    }
  }

  console.log("\nDone — message marked as paid on the relayer contract.");
}

main().catch((e) => {
  console.error("Error:", e.message ?? e);
  process.exit(1);
});
