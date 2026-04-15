import "dotenv/config";
import { ethers } from "ethers";

const DESTINATION_RPC_URL = process.env.DESTINATION_CHAIN_RPC_URL;
const SOURCE_RPC_URL = process.env.CREDITCOIN_RPC_URL;
const DESTINATION_PRIVATE_KEY = process.env.DESTINATION_CHAIN_PRIVATE_KEY;
const SOURCE_PRIVATE_KEY = process.env.CREDITCOIN_CHAIN_PRIVATE_KEY;
const DESTINATION_CONTRACT_ADDR = process.env.DESTINATION_CONTRACT_ADDR;
const DAPP_CONTRACT_ADDR = process.env.DAPP_CONTRACT_ADDR;

if (!DESTINATION_RPC_URL) throw new Error("Missing DESTINATION_CHAIN_RPC_URL");
if (!SOURCE_RPC_URL) throw new Error("Missing CREDITCOIN_RPC_URL");
if (!DESTINATION_PRIVATE_KEY) throw new Error("Missing DESTINATION_CHAIN_PRIVATE_KEY");
if (!SOURCE_PRIVATE_KEY) throw new Error("Missing CREDITCOIN_CHAIN_PRIVATE_KEY");
if (!DAPP_CONTRACT_ADDR) throw new Error("Missing DAPP_CONTRACT_ADDR");

const destinationAbi = [
  "event MessageReceived(bytes32 indexed messageId, address indexed emitter, bytes payload)"
];

const dappAbi = [
  "function markDelivered(bytes32 messageId) external"
];

async function main() {
  const destinationProvider = new ethers.JsonRpcProvider(DESTINATION_RPC_URL);
  const sourceProvider = new ethers.JsonRpcProvider(SOURCE_RPC_URL);

  const deliveryListenerWallet = new ethers.Wallet(DESTINATION_PRIVATE_KEY!, destinationProvider);
  const dappOwnerWallet = new ethers.Wallet(SOURCE_PRIVATE_KEY!, sourceProvider);

  const destination = new ethers.Contract(DESTINATION_CONTRACT_ADDR!, destinationAbi, deliveryListenerWallet);
  const dapp = new ethers.Contract(DAPP_CONTRACT_ADDR!, dappAbi, dappOwnerWallet);

  const seenMessageIds = new Set<string>();

  console.log("Listening for destination contract MessageReceived events...");
  console.log(`Destination contract: ${DESTINATION_CONTRACT_ADDR}`);
  console.log(`SimpleDApp: ${DAPP_CONTRACT_ADDR}`);

  destination.on("MessageReceived", async (messageId, emitter, payload, event) => {
    const id = String(messageId);

    if (seenMessageIds.has(id)) {
      console.log(`Skipping duplicate messageId: ${id}`);
      return;
    }

    seenMessageIds.add(id);

    console.log("MessageReceived");
    console.log(`  messageId: ${id}`);
    console.log(`  emitter:   ${emitter}`);
    console.log(`  payload:   ${payload}`);
    if (event?.log?.transactionHash) {
      console.log(`  txHash:    ${event.log.transactionHash}`);
    }

    try {
      const tx = await dapp.markDelivered(messageId);
      console.log(`markDelivered tx sent: ${tx.hash}`);

      const receipt = await tx.wait();
      console.log(`markDelivered confirmed in block ${receipt.blockNumber}`);
    } catch (err) {
      seenMessageIds.delete(id);
      console.error(`Failed to markDelivered for ${id}:`, err);
    }
  });

  process.on("SIGINT", async () => {
    console.log("Shutting down listener...");
    destination.removeAllListeners("MessageReceived");
    process.exit(0);
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});