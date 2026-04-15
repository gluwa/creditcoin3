import "dotenv/config";
import { ethers } from "ethers";

// --- Parse CLI args ---
function getArg(name: string): string | undefined {
  const idx = process.argv.indexOf(name);
  if (idx !== -1 && idx + 1 < process.argv.length) {
    return process.argv[idx + 1];
  }
  return undefined;
}

const message = getArg("--message") ?? "hello writability";
const requiresAck = getArg("--requiresAck") === "true" ? true : false;

// --- Env vars ---
const RPC_URL = process.env.CREDITCOIN_RPC_URL;
const PRIVATE_KEY = process.env.CREDITCOIN_CHAIN_PRIVATE_KEY;
const DAPP_ADDR = process.env.DAPP_CONTRACT_ADDR;
const DESTINATION_CONTRACT_ADDR = process.env.DESTINATION_CONTRACT_ADDR;

if (!RPC_URL) throw new Error("Missing CREDITCOIN_RPC_URL");
if (!PRIVATE_KEY) throw new Error("Missing CREDITCOIN_CHAIN_PRIVATE_KEY");
if (!DAPP_ADDR) throw new Error("Missing DAPP_CONTRACT_ADDR");
if (!DESTINATION_CONTRACT_ADDR) throw new Error("Missing DESTINATION_CONTRACT_ADDR");

// --- ABI ---
const abi = [
  "function publishMessage(bool requiresAck, address destinationContract, string message) external returns (bytes32)",
  "event MessageDelivered(bytes32 indexed messageId)"
];

// --- Main ---
async function main() {
  const provider = new ethers.JsonRpcProvider(RPC_URL);
  const wallet = new ethers.Wallet(PRIVATE_KEY!, provider);

  const contract = new ethers.Contract(DAPP_ADDR!, abi, wallet);

  // 🔊 Start listening for MessageDelivered
  contract.on("MessageDelivered", (messageId) => {
    console.log("📬 MessageDelivered event received!");
    console.log("🆔 messageId:", messageId);
  });

  console.log("👂 Listening for MessageDelivered events...");

  // --- Send message ---
  console.log("📤 Sending message...");
  console.log("Message:", message);
  console.log("requiresAck:", requiresAck);

  const destinationContract = DESTINATION_CONTRACT_ADDR!;
  const tx = await contract.publishMessage(
    requiresAck,
    destinationContract,
    message
  );

  console.log("Tx sent:", tx.hash);

  const receipt = await tx.wait();
  console.log("✅ Confirmed in block:", receipt.blockNumber);

  // Keep process alive to listen for events
  console.log("⏳ Waiting for MessageDelivered events...");
}

// Graceful shutdown
process.on("SIGINT", () => {
  console.log("👋 Shutting down...");
  process.exit(0);
});

main().catch((err) => {
  console.error(err);
  process.exit(1);
});