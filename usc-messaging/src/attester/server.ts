/**
 * Attester worker: listens for MessagePublished on Outbox (Source chain)
 * and MessageDelivered on Inbox (Destination chain).
 *
 * On MessagePublished: "votes" and forwards to relayer via HTTP.
 * On MessageDelivered: logs delivery (future: trigger ACK flow).
 *
 * Start: node dist/attester/server.js --outbox 0x... --inbox 0x...
 */

import { ethers } from "ethers";
import { loadAttesterConfig } from "./config.js";
import { listenOutbox, listenInbox, type StopFn } from "./listeners.js";
import { notifyRelayer } from "./relayer-client.js";
import type { PublishedMessage } from "./types.js";

/** Track messages that require ACK (messageId -> requiresAck) */
const ackTracker = new Map<string, boolean>();

async function main(): Promise<void> {
  const config = await loadAttesterConfig();

  const sourceProvider = new ethers.JsonRpcProvider(
    config.sourceRpcUrl,
  );
  const destinationProvider = new ethers.JsonRpcProvider(
    config.destinationRpcUrl,
  );

  const [sourceBlock, destinationBlock] = await Promise.all([
    sourceProvider.getBlockNumber(),
    destinationProvider.getBlockNumber(),
  ]);

  console.log(`Attester starting`);
  console.log(
    `  Source RPC: ${config.sourceRpcUrl} (block ${sourceBlock})`,
  );
  console.log(
    `  Destination RPC: ${config.destinationRpcUrl} (block ${destinationBlock})`,
  );
  console.log(`  Outbox: ${config.outboxAddress}`);
  console.log(`  Inbox: ${config.inboxAddress}`);
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
        `[MessagePublished] messageId=${msg.messageId} emitter=${msg.emitterAddress} requiresAck=${msg.requiresAck}`,
      );

      // Track for ACK if needed
      ackTracker.set(msg.messageId, msg.requiresAck);

      // POC: "vote" is implicit — forward directly to relayer
      await notifyRelayer(config.relayerUrl, msg);
    },
  );
  stopFns.push(stopOutbox);

  // Listen for MessageDelivered on Inbox (Destination chain)
  const stopInbox = listenInbox(
    destinationProvider,
    config.inboxAddress,
    destinationBlock,
    config.pollIntervalMs,
    async (delivered) => {
      console.log(
        `[MessageDelivered] messageId=${delivered.messageId} processor=${delivered.processor}`,
      );

      const requiresAck = ackTracker.get(delivered.messageId);
      if (requiresAck) {
        // TODO: call Outbox.acknowledgeMessage(messageId) — requires gas on Source chain
        console.log(
          `[ACK required] messageId=${delivered.messageId} — acknowledge not yet implemented`,
        );
      }
    },
  );
  stopFns.push(stopInbox);

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
