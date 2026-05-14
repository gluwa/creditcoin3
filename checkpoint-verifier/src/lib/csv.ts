import * as fs from 'fs';
import { Checkpoint } from '../types/checkpoint';

/**
 * Validates that a string is a valid 32-byte hex digest with 0x prefix.
 */
function isValidDigest(digest: string): boolean {
    return /^0x[a-fA-F0-9]{64}$/.test(digest);
}

/**
 * Parses a CSV file containing checkpoints.
 * Expected format: block_number,digest (no headers)
 * Each digest should be a 0x-prefixed 32-byte hex string.
 *
 * @param filePath Path to the CSV file
 * @returns Array of checkpoints sorted by block number
 * @throws Error if file cannot be read or contains invalid data
 */
export function parseCheckpointsCsv(filePath: string): Checkpoint[] {
    if (!fs.existsSync(filePath)) {
        throw new Error(`CSV file not found: ${filePath}`);
    }

    const content = fs.readFileSync(filePath, 'utf-8');
    const lines = content.split('\n').filter((line) => line.trim() !== '');

    const checkpoints: Checkpoint[] = [];

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i].trim();

        const parts = line.split(',');
        if (parts.length !== 2) {
            throw new Error(`Invalid CSV format at line ${i + 1}: expected "block_number,digest", got "${line}"`);
        }

        const blockNumber = parseInt(parts[0], 10);
        if (isNaN(blockNumber) || blockNumber < 0) {
            throw new Error(`Invalid block number at line ${i + 1}: "${parts[0]}"`);
        }

        const digest = parts[1].trim().toLowerCase();
        if (!isValidDigest(digest)) {
            throw new Error(
                `Invalid digest format at line ${i + 1}: expected 0x-prefixed 32-byte hex, got "${parts[1]}"`,
            );
        }

        checkpoints.push({ blockNumber, digest });
    }

    // Sort by block number ascending
    checkpoints.sort((a, b) => a.blockNumber - b.blockNumber);

    return checkpoints;
}

/**
 * Writes checkpoints to a CSV file.
 *
 * @param filePath Path to the output CSV file
 * @param checkpoints Array of checkpoints to write
 */
export function writeCheckpointsCsv(filePath: string, checkpoints: Checkpoint[]): void {
    const lines = checkpoints.map((cp) => `${cp.blockNumber},${cp.digest}`);
    fs.writeFileSync(filePath, lines.join('\n') + '\n', 'utf-8');
}
