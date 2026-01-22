/**
 * Pending blocks queue
 *
 * Manages blocks that have been received from the source chain
 * but are waiting for attestation on Creditcoin3.
 */

import type { BlockInfo } from '../types.ts';

/**
 * Queue for managing pending blocks waiting for attestation
 */
export class PendingBlockQueue {
  private blocks: Map<number, BlockInfo> = new Map();
  private maxSize: number;
  private highestAttestedBlock = 0;

  constructor(maxSize: number = 100) {
    this.maxSize = maxSize;
  }

  /**
   * Add a block to the pending queue
   */
  add(block: BlockInfo): void {
    // Skip blocks that have already been attested
    if (block.blockNumber <= this.highestAttestedBlock) {
      return;
    }

    // Skip if queue is full and this block is older than the oldest in queue
    if (this.blocks.size >= this.maxSize) {
      const oldestBlock = this.getOldestBlockNumber();
      if (oldestBlock !== null && block.blockNumber < oldestBlock) {
        return;
      }
      // Remove oldest to make room
      if (oldestBlock !== null) {
        this.blocks.delete(oldestBlock);
      }
    }

    this.blocks.set(block.blockNumber, block);
  }

  /**
   * Get blocks that have been attested (block number <= attestedUpTo)
   * and remove them from the queue
   */
  getAttestedBlocks(attestedUpTo: number): BlockInfo[] {
    const attested: BlockInfo[] = [];

    // Update highest attested block
    if (attestedUpTo > this.highestAttestedBlock) {
      this.highestAttestedBlock = attestedUpTo;
    }

    // Find all blocks that are now attested
    for (const [blockNumber, block] of this.blocks) {
      if (blockNumber <= attestedUpTo) {
        attested.push(block);
      }
    }

    // Remove attested blocks from queue
    for (const block of attested) {
      this.blocks.delete(block.blockNumber);
    }

    // Sort by block number
    attested.sort((a, b) => a.blockNumber - b.blockNumber);

    return attested;
  }

  /**
   * Get the current queue size
   */
  get size(): number {
    return this.blocks.size;
  }

  /**
   * Get the oldest block number in the queue
   */
  getOldestBlockNumber(): number | null {
    if (this.blocks.size === 0) {
      return null;
    }

    let oldest = Number.MAX_SAFE_INTEGER;
    for (const blockNumber of this.blocks.keys()) {
      if (blockNumber < oldest) {
        oldest = blockNumber;
      }
    }

    return oldest === Number.MAX_SAFE_INTEGER ? null : oldest;
  }
}
