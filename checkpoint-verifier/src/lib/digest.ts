import { JsonRpcProvider, WebSocketProvider } from 'ethers';

import { encoding, proofGenerator } from '@gluwa/usc-sdk';

import { getBlockWithReceipts } from './block-provider';
import { Checkpoint, VerificationResult, VerificationSummary } from '../types/checkpoint';

/**
 * Computes the merkle root and digest for a single block.
 *
 * @param provider RPC provider
 * @param blockNumber Block number to process
 * @param prevDigest Previous block's digest (0x-prefixed 32-byte hex string) or null if no previous digest
 * @returns The computed digest for this block, or null if block fetch failed
 */
export async function computeBlockDigest(
    provider: JsonRpcProvider | WebSocketProvider,
    blockNumber: number,
    prevDigest: string | null,
): Promise<{ root: string; digest: string } | null> {
    const blockData = await getBlockWithReceipts(provider, blockNumber);
    if (!blockData) {
        return null;
    }

    const { transactions, receipts } = blockData;

    // Sort by transaction index
    const orderedReceipts = receipts.sort((a, b) => a.index - b.index);
    const orderedTransactions = transactions.sort((a, b) => a.formatted.index - b.formatted.index);

    // Compute merkle root
    const merkleRoot = proofGenerator.merkle.computeMerkleRootOfBlock(
        orderedTransactions,
        orderedReceipts,
        encoding.EncodingVersion.V1,
    );

    // Compute digest
    const digest = proofGenerator.merkle.computeDigestOf(blockNumber, merkleRoot, prevDigest);

    return { root: merkleRoot, digest };
}

/**
 * Computes checkpoint digests for a range of blocks.
 *
 * @param provider RPC provider
 * @param startBlock Starting block number (inclusive)
 * @param endBlock Ending block number (inclusive)
 * @param startingDigest Starting digest (0x-prefixed 32-byte hex string) or null if no previous digest
 * @param onProgress Optional callback for progress updates
 * @returns The final checkpoint at endBlock
 */
export async function computeRangeDigest(
    provider: JsonRpcProvider | WebSocketProvider,
    startBlock: number,
    endBlock: number,
    startingDigest: string | null,
    onProgress?: (current: number, total: number) => void,
): Promise<Checkpoint> {
    let prevDigest = startingDigest;
    const totalBlocks = endBlock - startBlock + 1;

    for (let blockNumber = startBlock; blockNumber <= endBlock; blockNumber++) {
        const result = await computeBlockDigest(provider, blockNumber, prevDigest);
        if (!result) {
            throw new Error(`Failed to compute digest for block ${blockNumber}`);
        }

        prevDigest = result.digest;

        if (onProgress) {
            onProgress(blockNumber - startBlock + 1, totalBlocks);
        }
    }

    return {
        blockNumber: endBlock,
        digest: prevDigest || proofGenerator.merkle.ZERO_HASH, // If no blocks were processed, return ZERO_HASH
    };
}

/**
 * Verifies checkpoints from a CSV file by computing expected digests.
 *
 * @param provider RPC provider
 * @param checkpoints Array of checkpoints to verify
 * @param startingDigest Starting digest before the first checkpoint (0x-prefixed 32-byte hex string) or null if no previous digest
 * @param onProgress Optional callback for progress updates
 * @returns Verification summary with PASS/FAIL status for each checkpoint
 */
export async function verifyCheckpoints(
    provider: JsonRpcProvider | WebSocketProvider,
    checkpoints: Checkpoint[],
    startingDigest: string | null,
    onProgress?: (checkpointIndex: number, blockNumber: number, total: number) => void,
): Promise<VerificationSummary> {
    const results: VerificationResult[] = [];
    let prevDigest = startingDigest;
    let lastBlock = checkpoints.length > 0 ? checkpoints[0].blockNumber - 1 : -1;

    for (let i = 0; i < checkpoints.length; i++) {
        const checkpoint = checkpoints[i];

        if (onProgress) {
            onProgress(i + 1, checkpoint.blockNumber, checkpoints.length);
        }

        // Compute digest for all blocks from lastBlock+1 to checkpoint.blockNumber
        const startBlock = lastBlock + 1;

        try {
            const computed = await computeRangeDigest(provider, startBlock, checkpoint.blockNumber, prevDigest);

            const passed = computed.digest.toLowerCase() === checkpoint.digest.toLowerCase();
            results.push({
                blockNumber: checkpoint.blockNumber,
                passed,
                expected: checkpoint.digest,
                computed: computed.digest,
            });

            prevDigest = computed.digest;
        } catch (error) {
            console.error(
                `Error verifying checkpoint at block ${checkpoint.blockNumber}: ${(error as Error).message} exiting...`,
            );

            // On error, we return immediately with a failed result for this checkpoint and skip the rest
            return {
                total: results.length,
                passed: results.filter((r) => r.passed).length,
                failed: 1,
                results,
            };
        }

        lastBlock = checkpoint.blockNumber;
    }

    const passedCount = results.filter((r) => r.passed).length;
    const failedCount = results.length - passedCount;

    return {
        total: results.length,
        passed: passedCount,
        failed: failedCount,
        results,
    };
}
