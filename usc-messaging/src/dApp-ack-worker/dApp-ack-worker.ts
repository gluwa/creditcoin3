// ⚠️ DEMO / NON-PRODUCTION. This worker's delivery bookkeeping (`markDelivered`) is in-memory and
// NOT crash-durable: updates can be lost across downtime, retry exhaustion, or SIGTERM, and there is
// no durable cursor or classified-retry handling. It exists to drive the write-ability demo/e2e, not
// to run a production destination dApp. A production consumer needs a durable cursor/outbox,
// classified retries, and graceful retry draining (see audit P3-3).
import "dotenv/config";
import { ethers } from "ethers";
import { listenDestinationContract } from "./listeners.js";

const DESTINATION_RPC_URL = process.env.DESTINATION_CHAIN_RPC_URL;
const SOURCE_RPC_URL = process.env.CREDITCOIN_RPC_URL;
const DESTINATION_PRIVATE_KEY = process.env.DESTINATION_CHAIN_PRIVATE_KEY;
const SOURCE_PRIVATE_KEY = process.env.CREDITCOIN_CHAIN_PRIVATE_KEY;
const DESTINATION_CONTRACT_ADDR = process.env.DESTINATION_CONTRACT_ADDR;
const DAPP_CONTRACT_ADDR = process.env.DAPP_CONTRACT_ADDR;
const POLL_INTERVAL_MS = Number(
  process.env.DELIVERY_POLL_INTERVAL_MS ?? "2000",
);

if (!DESTINATION_RPC_URL) throw new Error("Missing DESTINATION_CHAIN_RPC_URL");
if (!SOURCE_RPC_URL) throw new Error("Missing CREDITCOIN_RPC_URL");
if (!DESTINATION_PRIVATE_KEY)
  throw new Error("Missing DESTINATION_CHAIN_PRIVATE_KEY");
if (!SOURCE_PRIVATE_KEY)
  throw new Error("Missing CREDITCOIN_CHAIN_PRIVATE_KEY");
if (!DESTINATION_CONTRACT_ADDR)
  throw new Error("Missing DESTINATION_CONTRACT_ADDR");
if (!DAPP_CONTRACT_ADDR) throw new Error("Missing DAPP_CONTRACT_ADDR");

const dappAbi = ["function markDelivered(bytes32 messageId) external"];

const MARK_DELIVERED_RETRY_MS = 5_000;
const MARK_DELIVERED_MAX_ATTEMPTS = 10;

// Retries live here (not via the listener cursor): the listener advances past the event's block
// as soon as the batch is dispatched, so a failed markDelivered would otherwise never be
// re-fetched and the ack would be silently lost.
async function markDeliveredWithRetry(
  dapp: ethers.Contract,
  messageId: string,
  attempt = 1,
): Promise<void> {
  try {
    const tx = await dapp.markDelivered(messageId);
    console.log(`markDelivered tx sent: ${tx.hash}`);

    const receipt = await tx.wait();
    console.log(`markDelivered confirmed in block ${receipt.blockNumber}`);
  } catch (err) {
    if (attempt >= MARK_DELIVERED_MAX_ATTEMPTS) {
      console.error(
        `Failed to markDelivered for ${messageId} after ${attempt} attempts — giving up:`,
        err,
      );
      return;
    }
    console.error(
      `Failed to markDelivered for ${messageId} (attempt ${attempt}), retrying in ${MARK_DELIVERED_RETRY_MS}ms:`,
      err,
    );
    setTimeout(() => {
      void markDeliveredWithRetry(dapp, messageId, attempt + 1);
    }, MARK_DELIVERED_RETRY_MS);
  }
}

async function main() {
  const destinationProvider = new ethers.JsonRpcProvider(DESTINATION_RPC_URL);
  const sourceProvider = new ethers.JsonRpcProvider(SOURCE_RPC_URL);

  const dappOwnerWallet = new ethers.Wallet(
    SOURCE_PRIVATE_KEY!,
    sourceProvider,
  );
  const dapp = new ethers.Contract(
    DAPP_CONTRACT_ADDR!,
    dappAbi,
    dappOwnerWallet,
  );

  const seenMessageIds = new Set<string>();

  console.log("Listening for destination contract MessageReceived events...");
  console.log(`Destination contract: ${DESTINATION_CONTRACT_ADDR}`);
  console.log(`SimpleDApp: ${DAPP_CONTRACT_ADDR}`);

  const startBlock = await destinationProvider.getBlockNumber();

  const stopListener = listenDestinationContract(
    destinationProvider,
    DESTINATION_CONTRACT_ADDR!,
    startBlock,
    POLL_INTERVAL_MS,
    async ({ messageId, emitter, payload, txHash }) => {
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
      if (txHash) {
        console.log(`  txHash:    ${txHash}`);
      }

      await markDeliveredWithRetry(dapp, messageId);
    },
  );

  process.on("SIGINT", () => {
    console.log("Shutting down listener...");
    stopListener();
    process.exit(0);
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
