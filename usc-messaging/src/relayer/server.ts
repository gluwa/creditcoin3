/**
 * Relayer server: receives voted messages from attesters via HTTP POST,
 * queues them, and delivers to SimpleInbox on a configurable interval.
 * A separate worker listens for MessageDelivered events on SimpleInbox and
 * acknowledges each one on the source chain Outbox.
 *
 * Start: node dist/relayer/server.js --inbox 0x... --outbox 0x...
 */

import { ethers } from "ethers";
import express from "express";
import { loadRelayerConfig } from "./config.js";
import { deliverMessage } from "./deliver.js";
import { listenInbox, type StopFn } from "./listeners.js";
import type { DeliveredMessage } from "./types.js";

const OUTBOX_ABI = ["function acknowledgeMessage(bytes32 messageId) public"];

/**
 * Calls Outbox.acknowledgeMessage on the source chain. Best-effort: errors
 * are logged but do not affect the already-completed delivery.
 */
async function acknowledgeOnSource(
  sourceSigner: ethers.Wallet,
  outboxAddress: string,
  messageId: string,
): Promise<void> {
  const outbox = new ethers.Contract(outboxAddress, OUTBOX_ABI, sourceSigner);
  try {
    const tx = await outbox.acknowledgeMessage(messageId);
    await tx.wait();
    console.log(`[ACK] messageId=${messageId} acknowledged on source chain`);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes("MessageAlreadyAcknowledged")) {
      console.warn(`[ACK] messageId=${messageId} already acknowledged`);
    } else {
      console.error(`[ACK] Failed to acknowledge messageId=${messageId}:`, msg);
    }
  }
}

async function main(): Promise<void> {
  const config = await loadRelayerConfig();

  const provider = new ethers.JsonRpcProvider(config.rpcUrl);
  const signer = new ethers.Wallet(config.privateKey, provider);

  const sourceProvider = new ethers.JsonRpcProvider(config.sourceRpcUrl);
  const sourceSigner = new ethers.Wallet(config.privateKey, sourceProvider);

  // Pending message store: map for O(1) deduplication, queue for ordered processing.
  const pendingMessages = new Map<string, DeliveredMessage>();
  const pendingQueue: string[] = [];

  // Delivery worker: processes all pending messages on each tick.
  const deliveryTimer = setInterval(async () => {
    if (pendingQueue.length === 0) return;
    console.log(
      `[Worker] Processing ${pendingQueue.length} pending message(s)`,
    );

    // Snapshot the queue so new arrivals during this tick are picked up next time.
    const snapshot = [...pendingQueue];

    for (const messageId of snapshot) {
      const msg = pendingMessages.get(messageId);
      if (!msg) continue; // already removed by a concurrent tick (safety guard)

      const result = await deliverMessage(
        provider,
        signer,
        config.inboxAddress,
        msg,
      );

      if (result.success) {
        console.log(
          `[Worker] Delivered messageId=${messageId} tx=${result.txHash}`,
        );
        pendingMessages.delete(messageId);
        pendingQueue.splice(pendingQueue.indexOf(messageId), 1);
      } else {
        console.error(
          `[Worker] Failed to deliver messageId=${messageId}: ${result.error} — will retry`,
        );
      }
    }
  }, config.deliveryIntervalMs);

  // ACK worker: polls SimpleInbox for MessageDelivered events and acknowledges
  // each one on the source chain Outbox.
  const destBlock = await provider.getBlockNumber();
  const stopInboxListener = listenInbox(
    provider,
    config.inboxAddress,
    destBlock,
    config.deliveryIntervalMs,
    async (messageId: string) => {
      console.log(`[Inbox] MessageDelivered messageId=${messageId}`);
      await acknowledgeOnSource(sourceSigner, config.outboxAddress, messageId);
    },
  );

  // HTTP: POST /deliver to receive messages from attesters.
  if (config.httpPort > 0) {
    const app = express();
    app.use(express.json());

    app.post("/deliver", (req, res) => {
      const msg = req.body as DeliveredMessage;

      if (!msg?.messageId || !msg?.emitterAddress || !msg?.payload) {
        res.status(400).json({
          error: "Missing required fields: messageId, emitterAddress, payload",
        });
        return;
      }

      if (pendingMessages.has(msg.messageId)) {
        res.status(202).json({ queued: false, reason: "duplicate" });
        return;
      }

      pendingMessages.set(msg.messageId, msg);
      pendingQueue.push(msg.messageId);
      console.log(
        `[HTTP] Queued messageId=${msg.messageId} (queue size: ${pendingQueue.length})`,
      );

      res.status(202).json({ queued: true, messageId: msg.messageId });
    });

    app.get("/health", (_req, res) =>
      res.json({ status: "ok", pending: pendingQueue.length }),
    );

    app.listen(config.httpPort, () => {
      console.log(
        `Relayer HTTP on http://localhost:${config.httpPort} (POST /deliver)`,
      );
    });
  }

  console.log(`Relayer starting`);
  console.log(`  Destination RPC: ${config.rpcUrl}`);
  console.log(`  Source RPC:      ${config.sourceRpcUrl}`);
  console.log(`  Inbox:           ${config.inboxAddress}`);
  console.log(`  Outbox:          ${config.outboxAddress}`);
  console.log(`  Delivery interval: ${config.deliveryIntervalMs}ms`);

  const stopFns: StopFn[] = [stopInboxListener];

  const shutdown = () => {
    console.log("\nRelayer shutting down...");
    clearInterval(deliveryTimer);
    for (const stop of stopFns) stop();
    process.exit(0);
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
