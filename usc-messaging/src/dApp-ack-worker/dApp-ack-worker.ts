import "dotenv/config";
import { ethers, Log } from "ethers";
import {
  listenDestinationContract,
  EVENT_TOKENS_BRIDGED,
  EVENT_TOKENS_BURNED_FOR_BRIDGING,
} from "./listeners.js";

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

const dappAbi = [
  "function markDelivered(bytes32 messageId) external",
  "function redeemTokens(uint256 amount, address recipient) external",
  "event TokensRedeemed(address indexed recipient, uint256 amount)"
];

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

  console.log("Listening for destination contract bridge events...");
  console.log(`Destination contract: ${DESTINATION_CONTRACT_ADDR}`);
  console.log(`SimpleDApp: ${DAPP_CONTRACT_ADDR}`);

  const startBlock = await destinationProvider.getBlockNumber();

  const stopListener = listenDestinationContract(
    destinationProvider,
    DESTINATION_CONTRACT_ADDR!,
    startBlock,
    POLL_INTERVAL_MS,
    async (event) => {
      if (event.eventName === EVENT_TOKENS_BRIDGED) {
        const id = event.messageId;

        if (seenMessageIds.has(id)) {
          console.log(`Skipping duplicate bridged messageId: ${id}`);
          return;
        }

        seenMessageIds.add(id);

        console.log("TokensBridged");
        console.log(`  messageId:      ${event.messageId}`);
        console.log(`  emitterAddress: ${event.emitterAddress}`);
        console.log(`  recipient:      ${event.recipient}`);
        console.log(`  amount:         ${event.amount}`);
        if (event.txHash) {
          console.log(`  txHash:         ${event.txHash}`);
        }

        try {
          const tx = await dapp.markDelivered(event.messageId);
          console.log(`markDelivered tx sent: ${tx.hash}`);

          const receipt = await tx.wait();
          console.log(`markDelivered confirmed in block ${receipt.blockNumber}`);
        } catch (err) {
          seenMessageIds.delete(id);
          console.error(`Failed to markDelivered for ${id}:`, err);
        }
      } else if (event.eventName === EVENT_TOKENS_BURNED_FOR_BRIDGING) {
        console.log("TokensBurnedForBridging");
        console.log(`  from:   ${event.from}`);
        console.log(`  amount: ${event.amount}`);
        if (event.txHash) {
          console.log(`  txHash: ${event.txHash}`);
        }

        try {
          const tx = await dapp.redeemTokens(event.amount, event.from);
          console.log(`redeemTokens tx sent: ${tx.hash}`);

          const receipt = await tx.wait();
          console.log(`redeemTokens confirmed in block ${receipt.blockNumber}`);

          const redeemedLog = receipt.logs.find((log: Log) => {
            try {
              const parsed = dapp.interface.parseLog(log);
              return parsed?.name === "TokensRedeemed";
            } catch {
              return false;
            }
          });

          if (!redeemedLog) {
            console.log("No TokensRedeemed event found in receipt");
            return;
          }

          const parsed = dapp.interface.parseLog(redeemedLog);
          if (!parsed) {
            console.log("Failed to parse TokensRedeemed log");
            return;
          }

          const recipient = String(parsed.args[0]);
          const amount = parsed.args[1].toString();

          console.log("TokensRedeemed event found");
          console.log(`  recipient: ${recipient}`);
          console.log(`  amount:    ${amount}`);
        } catch (err) {
          console.error(`Failed to redeemTokens for address: ${event.from}:`, err);
        }
      }
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
