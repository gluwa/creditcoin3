/**
 * Fetches block data from source chain for valid request generation.
 *
 * Uses ethers.js JsonRpcProvider (HTTP) to fetch a batch of recent blocks
 * with their transaction data at startup.
 */

import { JsonRpcProvider } from "ethers";
import type { BlockData } from "./types.ts";

const FETCH_TIMEOUT_MS = 30_000;
const BATCH_SIZE = 10;

// Fetch blocks this far behind the chain head so the proof-gen API
// has had time to ingest them (avoids BlockNotOnSourceChain errors).
const HEAD_OFFSET = 100;

/**
 * Fetch recent blocks with transactions from the source chain.
 *
 * @param rpcUrl - HTTP RPC URL for the source chain
 * @param count - Number of blocks to fetch (default: 50)
 * @param blockRange - Optional [start, end] block range
 * @returns Array of block data with transaction info
 */
export async function fetchBlocks(
  rpcUrl: string,
  count = 50,
  blockRange?: [number, number],
): Promise<BlockData[]> {
  const provider = new JsonRpcProvider(rpcUrl);

  let startBlock: number;
  let endBlock: number;

  if (blockRange) {
    [startBlock, endBlock] = blockRange;
    count = Math.min(count, endBlock - startBlock + 1);
  } else {
    const head = await provider.getBlockNumber();
    endBlock = head - HEAD_OFFSET;
    startBlock = Math.max(0, endBlock - count + 1);
    console.log(
      `Source chain head: ${head}, fetching ${HEAD_OFFSET} blocks behind head to avoid BlockNotOnSourceChain errors`,
    );
  }

  console.log(
    `Fetching ${count} blocks (${startBlock} to ${endBlock})...`,
  );

  const blocks: BlockData[] = [];
  const blockNumbers: number[] = [];

  for (let i = startBlock; i <= endBlock && blockNumbers.length < count; i++) {
    blockNumbers.push(i);
  }

  // Fetch in parallel batches
  for (let i = 0; i < blockNumbers.length; i += BATCH_SIZE) {
    const batch = blockNumbers.slice(i, i + BATCH_SIZE);
    const results = await Promise.allSettled(
      batch.map((num) => fetchOneBlock(provider, num)),
    );

    for (const result of results) {
      if (result.status === "fulfilled" && result.value) {
        blocks.push(result.value);
      }
    }

    const done = Math.min(i + BATCH_SIZE, blockNumbers.length);
    console.log(`  Fetched ${done}/${blockNumbers.length} blocks...`);
  }

  provider.destroy();

  const withTxs = blocks.filter((b) => b.txCount > 0);
  console.log(
    `Fetched ${blocks.length} blocks (${withTxs.length} with transactions)`,
  );

  return blocks;
}

function fetchOneBlock(
  provider: JsonRpcProvider,
  blockNumber: number,
): Promise<BlockData | null> {
  const timeout = new Promise<never>((_, reject) =>
    setTimeout(
      () => reject(new Error(`Timeout fetching block ${blockNumber}`)),
      FETCH_TIMEOUT_MS,
    )
  );

  const fetcher = (async () => {
    const block = await provider.getBlock(blockNumber, true);
    if (!block) return null;

    const txHashes: string[] = [];
    if (block.prefetchedTransactions?.length > 0) {
      for (const tx of block.prefetchedTransactions) {
        txHashes.push(tx.hash);
      }
    } else if (Array.isArray(block.transactions)) {
      for (const tx of block.transactions) {
        if (typeof tx === "string") {
          txHashes.push(tx);
        }
      }
    }

    return {
      blockNumber: block.number,
      txCount: txHashes.length,
      txHashes,
    };
  })();

  return Promise.race([fetcher, timeout]);
}
