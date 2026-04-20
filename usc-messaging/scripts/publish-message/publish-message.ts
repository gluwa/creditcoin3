import "dotenv/config";
import { ethers } from "ethers";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { EVENT_MESSAGE_DISPATCHED, EVENT_MESSAGE_DELIVERED, listenDAppContract } from "./listeners";
import { TEST_TOKEN_AMOUNT } from "../../src/consts";
import { requireEnv } from "../../src/utils";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);



// --- Parse CLI args ---
function getArg(name: string): string | undefined {
  const idx = process.argv.indexOf(name);
  if (idx !== -1 && idx + 1 < process.argv.length) {
    return process.argv[idx + 1];
  }
  return undefined;
}

const requiresAck = getArg("--requiresAck") === "true";

// --- Env vars ---
const CREDITCOIN_RPC_URL = requireEnv("CREDITCOIN_RPC_URL");
const CREDITCOIN_CHAIN_PRIVATE_KEY = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");
const DESTINATION_CHAIN_PUBLIC_KEY = requireEnv("DESTINATION_CHAIN_PUBLIC_KEY");
const DAPP_CONTRACT_ADDR = requireEnv("DAPP_CONTRACT_ADDR");
const DESTINATION_CONTRACT_ADDR = requireEnv("DESTINATION_CONTRACT_ADDR");
const POLL_INTERVAL_MS = Number(process.env.DAPP_POLL_INTERVAL_MS ?? "2000");

// --- ABI ---
const abi = [
  "function sendTokens(bool requiresAck, address destinationContract, address recipient, uint256 amount) external returns (bytes32)",
  "event MessageDelivered(bytes32 indexed messageId)",
  "event MessageDispatched(bytes32 indexed messageId)",
];

function runRequestAndRelayQuote(messageId: string): void {
  const scriptPath = path.resolve(__dirname, "../request-and-relay-quote.ts");

  console.log(`🚀 Starting request-and-relay-quote for messageId=${messageId}`);

  const child = spawn(
    "npx",
    ["tsx", scriptPath, "--message-id", messageId],
    {
      stdio: "inherit",
      shell: true,
      env: process.env,
    },
  );

  child.on("exit", (code) => {
    if (code === 0) {
      console.log("✅ request-and-relay-quote completed successfully");
    } else {
      console.error(`❌ request-and-relay-quote exited with code ${code}`);
    }
  });

  child.on("error", (err) => {
    console.error("❌ Failed to start request-and-relay-quote:", err);
  });
}

async function main() {
  const provider = new ethers.JsonRpcProvider(CREDITCOIN_RPC_URL);
  const wallet = new ethers.Wallet(CREDITCOIN_CHAIN_PRIVATE_KEY!, provider);
  const contract = new ethers.Contract(DAPP_CONTRACT_ADDR!, abi, wallet);

  const startBlock = await provider.getBlockNumber();
  let relayStarted = false;

  let messageIdStore: string;
  const stopDispatched = listenDAppContract(
    provider,
    DAPP_CONTRACT_ADDR!,
    startBlock,
    POLL_INTERVAL_MS,
    EVENT_MESSAGE_DISPATCHED,
    async ({ messageId }) => {
      console.log("📬 MessageDispatched event received!");
      console.log("🆔 messageId:", messageId);

      if (!relayStarted) {
        relayStarted = true;
        runRequestAndRelayQuote(messageId);
      }

      messageIdStore = messageId;
    },
  );

  const stopDelivered = listenDAppContract(
    provider,
    DAPP_CONTRACT_ADDR!,
    startBlock,
    POLL_INTERVAL_MS,
    EVENT_MESSAGE_DELIVERED,
    async ({ messageId }) => {
      // We only care about the delivery of a message with a matching id
      if (messageIdStore && messageIdStore == messageId) {
        console.log("📬 MessageDelivered event received!");
        console.log("🆔 messageId:", messageId);

        clearInterval(heartbeat); // ✅ stop the periodic log
      }
    },
  );

  console.log("👂 Polling for MessageDispatched and MessageDelivered events...");

  console.log("📤 Sending tokens...");
  console.log("Recipient: ", DESTINATION_CHAIN_PUBLIC_KEY);
  console.log("Amount: ", TEST_TOKEN_AMOUNT);
  console.log("requiresAck: ", requiresAck);

  const tx = await contract.sendTokens(
    requiresAck,
    DESTINATION_CONTRACT_ADDR,
    DESTINATION_CHAIN_PUBLIC_KEY,
    TEST_TOKEN_AMOUNT
  );

  console.log("Tx sent:", tx.hash);

  const receipt = await tx.wait();
  console.log("✅ Confirmed in block:", receipt.blockNumber);

  const heartbeat = setInterval(() => {
    console.log("⏳ Waiting for MessageDelivered...");
  }, 10_000);

  process.on("SIGINT", () => {
    console.log("👋 Shutting down...");
    stopDispatched();
    stopDelivered();
    process.exit(0);
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
