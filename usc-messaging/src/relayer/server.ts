/**
 * Relayer client: listens for ready messages and delivers to DummyInbox.
 * Mock P2P: reads from JSON file and/or HTTP POST.
 *
 * Start: node dist/relayer/server.js --inbox 0x... [--rpc-url http://...]
 */

import { ethers } from "ethers";
import express from "express";
import { readFile, writeFile } from "fs/promises";
import { existsSync } from "fs";
import { loadRelayerConfig, type RelayerConfig } from "./config.js";
import { deliverMessage } from "./deliver.js";
import type { ReadyMessage } from "./types.js";

async function loadMessages(filePath: string): Promise<ReadyMessage[]> {
  if (!existsSync(filePath)) return [];
  const raw = await readFile(filePath, "utf-8");
  const data = JSON.parse(raw);
  return Array.isArray(data) ? data : data.messages ?? [];
}

async function saveMessages(filePath: string, messages: ReadyMessage[]): Promise<void> {
  await writeFile(filePath, JSON.stringify({ messages }, null, 2));
}

async function processMessages(
  provider: ethers.Provider,
  signer: ethers.Signer,
  inboxAddress: string,
  filePath: string
): Promise<void> {
  const messages = await loadMessages(filePath);
  if (messages.length === 0) return;

  const remaining: ReadyMessage[] = [];
  for (const msg of messages) {
    const result = await deliverMessage(provider, signer, inboxAddress, msg);
    if (result.success) {
      console.log(`Delivered messageId=${msg.messageId} tx=${result.txHash}`);
    } else {
      console.error(`Failed to deliver messageId=${msg.messageId}: ${result.error}`);
      remaining.push(msg);
    }
  }
  await saveMessages(filePath, remaining);
}

async function main(): Promise<void> {
  const config = await loadRelayerConfig();
  const provider = new ethers.JsonRpcProvider(config.rpcUrl);
  const signer = new ethers.Wallet(config.privateKey, provider);

  // File watcher
  const run = async () => {
    try {
      await processMessages(provider, signer, config.inboxAddress, config.messagesFilePath);
    } catch (err) {
      console.error("Process error:", err);
    }
    setTimeout(run, config.pollIntervalMs);
  };
  run();

  // HTTP: POST /deliver for immediate delivery
  if (config.httpPort > 0) {
    const app = express();
    app.use(express.json());
    app.post("/deliver", async (req, res) => {
      const msg = req.body as ReadyMessage;
      if (!msg?.messageId || !msg?.emitterAddress || !msg?.destinationContract) {
        res.status(400).json({ error: "Missing messageId, emitterAddress, or destinationContract" });
        return;
      }
      msg.payloadData = msg.payloadData ?? "0x";
      const result = await deliverMessage(provider, signer, config.inboxAddress, msg);
      if (result.success) {
        res.json({ success: true, txHash: result.txHash });
      } else {
        res.status(500).json({ success: false, error: result.error });
      }
    });
    app.get("/health", (_req, res) => res.json({ status: "ok" }));
    app.listen(config.httpPort, () => {
      console.log(`Relayer HTTP on http://localhost:${config.httpPort} (POST /deliver)`);
    });
  }

  console.log(`Relayer watching ${config.messagesFilePath} every ${config.pollIntervalMs}ms`);
  console.log(`Inbox: ${config.inboxAddress}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
