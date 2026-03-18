/**
 * Proof Traffic Simulator
 *
 * A Deno-based tool that simulates proof query traffic by:
 * 1. Streaming blocks from source chain (Sepolia)
 * 2. Queueing blocks until they are attested on Creditcoin3
 * 3. Submitting proofs for random transactions once blocks are attested
 */

// Suppress Deno Node.js compatibility warnings
const originalWarn = console.warn;
console.warn = (...args: unknown[]) => {
  const msg = args[0];
  if (typeof msg === "string") {
    // Filter out Deno's Node.js compatibility warnings
    if (msg.includes("Not implemented: ClientRequest")) return;
  }
  originalWarn.apply(console, args);
};

import { loadConfig, logConfig } from "./config.ts";
import { BlockSubscriber } from "./subscribers/blockSubscriber.ts";
import { AttestationSubscriber } from "./subscribers/attestationSubscriber.ts";
import { PendingBlockQueue } from "./queue/pendingQueue.ts";
import { submitSingleProof } from "./submitter/singleSubmitter.ts";
import { submitBatchProofs } from "./submitter/batchSubmitter.ts";
import { startHealthServer } from "./server.ts";
import { setVerbose } from "./logger.ts";
import { SINGLE_SUBMISSION_DELAY_MS } from "./constants.ts";
import { sleep } from "./utils/reconnect.ts";
import type {
  BlockInfo,
  HealthStatus,
  Metrics,
  SimulatorConfig,
  TxInfo,
} from "./types.ts";

// Global state
let config: SimulatorConfig;
let blockSubscriber: BlockSubscriber;
let attestationSubscriber: AttestationSubscriber;
let pendingQueue: PendingBlockQueue;
let healthServer: { shutdown: () => void };
let isShuttingDown = false;
const startTime = Date.now();
let lastSingleSubmissionBlock: number | null = null;

// Metrics
const metrics = {
  blocksQueued: 0,
  blocksProcessed: 0,
  proofsSubmitted: 0,
  singleSubmissions: 0,
  batchSubmissions: 0,
  proofErrors: 0,
};

let lastError: string | null = null;
const uniqueErrors: Map<string, number> = new Map();

/**
 * Normalize an error message by stripping variable parts (URLs, block/tx numbers)
 * so that errors differing only in parameters are grouped together.
 */
function normalizeError(error: string): string {
  return error
    // Replace full URLs with just the domain + "..."
    .replace(
      /https?:\/\/[^\s:)]+/g,
      (url) => {
        try {
          const parsed = new URL(url);
          return `${parsed.origin}/...`;
        } catch {
          return url;
        }
      },
    )
    // Replace standalone hex strings (tx hashes, addresses)
    .replace(/\b0x[0-9a-fA-F]{8,}\b/g, "0x...")
    // Replace standalone numbers that look like block/tx numbers (4+ digits)
    .replace(/\b\d{4,}\b/g, "N");
}

/**
 * Record an error, tracking unique error messages and their occurrence count.
 * Errors are normalized to group messages that differ only in variable parts.
 */
function recordError(error: string): void {
  lastError = error;
  const key = normalizeError(error);
  uniqueErrors.set(key, (uniqueErrors.get(key) ?? 0) + 1);
}

/**
 * Get current health status
 */
function getHealthStatus(): HealthStatus {
  return {
    sepoliaConnected: blockSubscriber?.isConnected ?? false,
    cc3Connected: attestationSubscriber?.isConnected ?? false,
    sourceChainKey: config?.chainKey ?? 1, // default to sepolia
    cc3WsUrl: config?.cc3WsUrl ?? "",
    queueSize: pendingQueue?.size ?? 0,
    blocksProcessed: metrics.blocksProcessed,
    proofsSubmitted: metrics.proofsSubmitted,
    singleSubmissions: metrics.singleSubmissions,
    batchSubmissions: metrics.batchSubmissions,
    proofErrors: metrics.proofErrors,
    lastError,
    uniqueErrors: Object.fromEntries(uniqueErrors),
    uptimeSeconds: Math.floor((Date.now() - startTime) / 1000),
  };
}

/**
 * Get current metrics
 */
function getMetrics(): Metrics {
  return {
    ...metrics,
    queueSize: pendingQueue?.size ?? 0,
    sepoliaConnected: blockSubscriber?.isConnected ? 1 : 0,
    cc3Connected: attestationSubscriber?.isConnected ? 1 : 0,
  };
}

/**
 * Handle new block from source chain
 */
function handleNewBlock(block: BlockInfo): void {
  if (isShuttingDown) return;

  // Skip blocks with no transactions
  if (block.txHashes.length === 0) {
    return;
  }

  pendingQueue.add(block);
  metrics.blocksQueued++;

  console.log(
    `📦 Block ${block.blockNumber} queued (${block.txHashes.length} txs, queue: ${pendingQueue.size})`,
  );
}

/**
 * Handle attestation event
 */
async function handleAttestation(attestedBlock: number): Promise<void> {
  if (isShuttingDown) return;

  // The latest attested block cannot be proven until the next attestation arrives.
  const provableUpTo = Math.max(attestedBlock - 1, 0);

  // Get all blocks that are now provable
  const attestedBlocks = pendingQueue.getAttestedBlocks(provableUpTo);

  if (attestedBlocks.length === 0) {
    if (provableUpTo > 0) {
      console.log(
        `ℹ️  Attested ${attestedBlock}, nothing provable yet (waiting for next attestation)`,
      );
    }
    return;
  }

  console.log(
    `\n🎯 ${attestedBlocks.length} block(s) provable up to block ${provableUpTo} (latest attested: ${attestedBlock})`,
  );

  const batchTxInfos: TxInfo[] = [];
  const singleTxInfos: TxInfo[] = [];

  // Select transactions per block
  for (const block of attestedBlocks) {
    metrics.blocksProcessed++;

    if (block.txHashes.length === 0) {
      continue;
    }

    const useBatch = Math.random() < config.batchProbability;
    const txInfos = selectTxInfosForBlock(block, useBatch);

    if (useBatch) {
      batchTxInfos.push(...txInfos);
    } else {
      if (txInfos.length > 0) {
        singleTxInfos.push(...txInfos);
        lastSingleSubmissionBlock = block.blockNumber;
      }
    }
  }

  // Batch submissions (can include multiple blocks sharing continuity)
  if (batchTxInfos.length > 0) {
    try {
      const result = await submitBatchProofs(config, batchTxInfos);
      metrics.batchSubmissions += result.batches;
      metrics.proofsSubmitted += result.successful;
      metrics.proofErrors += result.failed;
    } catch (error) {
      metrics.proofErrors++;
      const errorMsg = error instanceof Error ? error.message : String(error);
      recordError(errorMsg);
      console.error("❌ Error processing batch submissions:", errorMsg);
    }
  }

  // Single submissions (one per block)
  for (const txInfo of singleTxInfos) {
    metrics.singleSubmissions++;
    const result = await submitSingleProof(config, txInfo);
    if (result.success) {
      metrics.proofsSubmitted++;
    } else {
      metrics.proofErrors++;
      recordError(result.error ?? "Unknown error");
    }

    // Small delay between single submissions
    if (!isShuttingDown) {
      await sleep(SINGLE_SUBMISSION_DELAY_MS);
    }
  }
}

/** Select transactions for a block. Batch mode: 1–2 txs per block; single: 1 tx. */
function selectTxInfosForBlock(block: BlockInfo, useBatch: boolean): TxInfo[] {
  let selectedTxs: Array<{ txHash: string; txIndex: number }>;
  if (useBatch) {
    const maxPerBlock = block.txHashes.length >= 2 ? 2 : 1;
    selectedTxs = selectRandomTransactions(block.txHashes, maxPerBlock);
  } else {
    if (!shouldSubmitSingleForBlock(block.blockNumber)) {
      console.log(
        `⏭️  Skipping single submission for block ${block.blockNumber} (interval ${config.singleEveryBlocks})`,
      );
      return [];
    }
    selectedTxs = selectRandomTransactions(block.txHashes, 1);
  }

  const txInfos = selectedTxs.map((tx) => ({
    txHash: tx.txHash,
    txIndex: tx.txIndex,
    blockNumber: block.blockNumber,
  }));

  console.log(
    `📋 Block ${block.blockNumber}: selected ${txInfos.length} of ${block.txHashes.length} transactions (${
      useBatch ? "batch" : "single"
    })`,
  );

  return txInfos;
}

function shouldSubmitSingleForBlock(blockNumber: number): boolean {
  if (config.singleEveryBlocks <= 1) {
    return true;
  }
  if (lastSingleSubmissionBlock === null) {
    return true;
  }
  return blockNumber - lastSingleSubmissionBlock >= config.singleEveryBlocks;
}

/**
 * Select random transactions from a list
 */
function selectRandomTransactions(
  txHashes: string[],
  count: number,
): Array<{ txHash: string; txIndex: number }> {
  if (count >= txHashes.length) {
    return txHashes.map((txHash, txIndex) => ({ txHash, txIndex }));
  }

  const selected: Array<{ txHash: string; txIndex: number }> = [];
  const availableIndices = txHashes.map((_, index) => index);

  while (selected.length < count && availableIndices.length > 0) {
    const index = Math.floor(Math.random() * availableIndices.length);
    const txIndex = availableIndices.splice(index, 1)[0];
    selected.push({ txHash: txHashes[txIndex], txIndex });
  }

  return selected;
}

/**
 * Graceful shutdown handler
 */
async function shutdown(): Promise<void> {
  if (isShuttingDown) return;
  isShuttingDown = true;

  console.log("\n⏳ Shutting down gracefully...");

  // Stop health server
  try {
    healthServer?.shutdown();
  } catch {
    // Ignore
  }

  // Stop subscribers
  await Promise.allSettled([
    blockSubscriber?.stop(),
    attestationSubscriber?.stop(),
  ]);

  console.log("\n📊 Final statistics:");
  console.log(`   Blocks queued: ${metrics.blocksQueued}`);
  console.log(`   Blocks processed: ${metrics.blocksProcessed}`);
  console.log(`   Proofs submitted: ${metrics.proofsSubmitted}`);
  console.log(`   Single submissions: ${metrics.singleSubmissions}`);
  console.log(`   Batch submissions: ${metrics.batchSubmissions}`);
  console.log(`   Errors: ${metrics.proofErrors}`);
  console.log(`   Uptime: ${Math.floor((Date.now() - startTime) / 1000)}s`);

  console.log("\n✅ Shutdown complete");
  Deno.exit(0);
}

/**
 * Main entry point
 */
async function main(): Promise<void> {
  console.log("🚀 Proof Traffic Simulator");
  console.log("==========================\n");

  try {
    // Load configuration
    config = loadConfig();
    logConfig(config);
    setVerbose(config.logVerbose);

    // Initialize components
    pendingQueue = new PendingBlockQueue(config.maxQueueSize);

    // Start health server
    healthServer = startHealthServer(
      config.healthPort,
      getHealthStatus,
      getMetrics,
    );

    // Create subscribers
    blockSubscriber = new BlockSubscriber(config.sourceRpcUrl, handleNewBlock);
    attestationSubscriber = new AttestationSubscriber(
      config.cc3WsUrl,
      config.chainKey,
      handleAttestation,
    );

    // Set up signal handlers
    Deno.addSignalListener("SIGINT", () => {
      console.log("\nReceived SIGINT");
      shutdown();
    });

    Deno.addSignalListener("SIGTERM", () => {
      console.log("\nReceived SIGTERM");
      shutdown();
    });

    // Start subscribers
    console.log("\n🔄 Starting subscribers...\n");
    await Promise.all([
      blockSubscriber.start(),
      attestationSubscriber.start(),
    ]);

    console.log("\n✅ Simulator running. Press Ctrl+C to stop.\n");

    // Keep process running
    await new Promise(() => {});
  } catch (error) {
    console.error("❌ Fatal error:", error);
    await shutdown();
    Deno.exit(1);
  }
}

// Run
main();
