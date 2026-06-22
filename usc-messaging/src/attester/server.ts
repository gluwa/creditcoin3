/**
 * Attester worker: listens for MessagePublished on Outbox (Source chain)
 * and forwards each message to the relayer via HTTP.
 *
 * Start: node dist/attester/server.js --outbox 0x...
 */

import { ethers } from "ethers";
import { loadAttesterConfig } from "./config.js";
import { listenOutbox, type StopFn } from "./listeners.js";
import { notifyRelayer } from "./relayer-client.js";
import type { PublishedMessage } from "./types.js";

async function main(): Promise<void> {
  const config = await loadAttesterConfig();

  const wallet = new ethers.Wallet(config.key);
  console.log(
    `Derived public key ${wallet.address} from provided private key.`,
  );

  console.log(`Connecting to source RPC at ${config.sourceRpcUrl}...`);
  const sourceProvider = new ethers.JsonRpcProvider(config.sourceRpcUrl);
  const sourceBlock = await sourceProvider.getBlockNumber();

  console.log(`Attester starting`);
  console.log(`  Source RPC: ${config.sourceRpcUrl} (block ${sourceBlock})`);
  console.log(`  Outbox: ${config.outboxAddress}`);
  console.log(`  Relayer: ${config.relayerUrl}`);
  console.log(`  Poll interval: ${config.pollIntervalMs}ms`);

  const stopFns: StopFn[] = [];

  // Listen for MessagePublished on Outbox (Source chain)
  const stopOutbox = listenOutbox(
    sourceProvider,
    config.outboxAddress,
    sourceBlock,
    config.pollIntervalMs,
    async (msg: PublishedMessage) => {
      console.log(
        `[Outbox] MessagePublished messageId=${msg.messageId} emitter=${msg.emitterAddress}`,
      );

      // POC: We "vote" on the message by signing its ID with our private key.
      const vote = await wallet.signMessage(msg.messageId);
      const votes = [vote];

      const deliveryMsg = {
        messageId: msg.messageId,
        emitterAddress: msg.emitterAddress,
        payload: msg.payload,
        requiresAck: msg.requiresAck,
        signedVotes: votes,
      };

      // Forward signed message to relayer
      await notifyRelayer(config.relayerUrl, deliveryMsg);
    },
  );
  stopFns.push(stopOutbox);

  // Graceful shutdown
  const shutdown = () => {
    console.log("\nAttester shutting down...");
    for (const stop of stopFns) {
      stop();
    }
    process.exit(0);
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
