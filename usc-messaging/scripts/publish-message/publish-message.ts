import "dotenv/config";
import { ethers } from "ethers";
import { EVENT_MESSAGE_DISPATCHED, EVENT_MESSAGE_DELIVERED, listenDAppContract } from "./listeners";

// --- Parse CLI args ---
function getArg(name: string): string | undefined {
  const idx = process.argv.indexOf(name);
  if (idx !== -1 && idx + 1 < process.argv.length) {
    return process.argv[idx + 1];
  }
  return undefined;
}

const message = getArg("--message") ?? "hello writability";
const requiresAck = getArg("--requiresAck") === "true";

// --- Env vars ---
const RPC_URL = process.env.CREDITCOIN_RPC_URL;
const PRIVATE_KEY = process.env.CREDITCOIN_CHAIN_PRIVATE_KEY;
const DAPP_ADDR = process.env.DAPP_CONTRACT_ADDR;
const DESTINATION_CONTRACT_ADDR = process.env.DESTINATION_CONTRACT_ADDR;
const POLL_INTERVAL_MS = Number(process.env.DAPP_POLL_INTERVAL_MS ?? "2000");

if (!RPC_URL) throw new Error("Missing CREDITCOIN_RPC_URL");
if (!PRIVATE_KEY) throw new Error("Missing CREDITCOIN_CHAIN_PRIVATE_KEY");
if (!DAPP_ADDR) throw new Error("Missing DAPP_CONTRACT_ADDR");
if (!DESTINATION_CONTRACT_ADDR) {
  throw new Error("Missing DESTINATION_CONTRACT_ADDR");
}

// --- ABI ---
const abi = [
  "function publishMessage(bool requiresAck, address destinationContract, string message) external returns (bytes32)",
  "event MessageDelivered(bytes32 indexed messageId)",
  "event MessageDispatched(bytes32 indexed messageId)",
];

async function main() {
  const provider = new ethers.JsonRpcProvider(RPC_URL);
  const wallet = new ethers.Wallet(PRIVATE_KEY!, provider);
  const contract = new ethers.Contract(DAPP_ADDR!, abi, wallet);

  const startBlock = await provider.getBlockNumber();

  let messageIdStore: string;
  const stopDispatched = listenDAppContract(
    provider,
    DAPP_ADDR!,
    startBlock,
    POLL_INTERVAL_MS,
    EVENT_MESSAGE_DISPATCHED,
    async ({ messageId }) => {
      console.log("📬 MessageDispatched event received!");
      console.log("🆔 messageId:", messageId);

      messageIdStore = messageId;
    },
  );

  const stopDelivered = listenDAppContract(
    provider,
    DAPP_ADDR!,
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

  console.log("📤 Sending message...");
  console.log("Message:", message);
  console.log("requiresAck:", requiresAck);

  const tx = await contract.publishMessage(
    requiresAck,
    DESTINATION_CONTRACT_ADDR,
    message,
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
