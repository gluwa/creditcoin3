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
 *
 * Options (env var / CLI flag):
 *   MESSAGE_ID          / --message-id (-m)        bytes32 hex (default: 0x…01)
 *   QUOTER_URL          / --quoter-url (-q)        quoter base URL
 *   RELAYER_CONTRACT     / --relayer-contract (-c)  SimpleRelayer address
 *   RPC_URL             / --rpc-url (-r)           destination chain RPC
 *   SENDER_PRIVATE_KEY  / --private-key (-k)       tx signer key
 *   DESTINATION_CHAIN_ID / --chain-id (-d)         destination chain ID
 *   REQUIRES_ACK        / --requires-ack (-a)      true/false
 */

import "dotenv/config";
import { ethers } from "ethers";
import { readFile } from "fs/promises";
import { existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

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
const DEFAULT_QUOTER_URL = `http://127.0.0.1:${process.env.QUOTER_PORT ?? "3300"}`;
const DEFAULT_RPC_URL = process.env.RELAYER_RPC_URL ?? "http://127.0.0.1:8545";
const DEFAULT_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"; // Anvil #0

const messageId =
  cliArg("message-id", "m") ?? process.env.MESSAGE_ID ?? DEFAULT_MESSAGE_ID;
const quoterUrl =
  cliArg("quoter-url", "q") ?? process.env.QUOTER_URL ?? DEFAULT_QUOTER_URL;
const rpcUrl =
  cliArg("rpc-url", "r") ?? process.env.RPC_URL ?? DEFAULT_RPC_URL;
const senderKey =
  cliArg("private-key", "k") ??
  process.env.SENDER_PRIVATE_KEY ??
  DEFAULT_PRIVATE_KEY;
const destinationChainId =
  cliArg("chain-id", "d") ?? process.env.DESTINATION_CHAIN_ID;

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

async function loadRelayerAddress(): Promise<string> {
  const fromCli = cliArg("relayer-contract", "c");
  if (fromCli) return fromCli;
  if (process.env.RELAYER_CONTRACT) return process.env.RELAYER_CONTRACT;

  const deployPath = join(ROOT, "deployments.json");
  if (existsSync(deployPath)) {
    const d = JSON.parse(await readFile(deployPath, "utf-8"));
    if (d.relayer) return d.relayer;
  }

  console.error(
    "No relayer contract address found. Provide via --relayer-contract, RELAYER_CONTRACT env, or deployments.json",
  );
  process.exit(1);
}

async function fetchQuote(baseUrl: string): Promise<QuoteResponse> {
  const params = new URLSearchParams({ messageId });
  if (destinationChainId) params.set("destinationChainId", destinationChainId);

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
  // 1. Load relayer contract address
  const relayerAddress = await loadRelayerAddress();
  console.log(`SimpleRelayer contract: ${relayerAddress}`);

  // 2. Request a quote from the quoter service
  const quote = await fetchQuote(quoterUrl);
  console.log("\nReceived quote:");
  console.log(`  messageId:            ${quote.messageId}`);
  console.log(`  relayPrice:           ${quote.relayPrice}`);
  console.log(`  acknowledgmentPrice:  ${quote.acknowledgmentPrice}`);
  console.log(`  payeeAddress:         ${quote.payeeAddress}`);
  console.log(`  paymentToken:         ${quote.paymentToken}`);
  console.log(`  expiry:               ${quote.expiry}`);
  console.log(`  signature:            ${quote.signature.slice(0, 20)}...`);

  // 3. Connect to the destination chain
  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const wallet = new ethers.Wallet(senderKey, provider);
  const relayer = new ethers.Contract(
    relayerAddress,
    SIMPLE_RELAYER_ABI,
    wallet,
  );

  // 4. Check if the quoter is registered
  const quoterSigner = ethers.computeAddress(
    process.env.QUOTER_SIGNER_PRIVATE_KEY ?? DEFAULT_PRIVATE_KEY,
  );
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

  // 5. Submit the quote to SimpleRelayer.validateAndCollectFee
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

  // 6. Check for emitted events
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
