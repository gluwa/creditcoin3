/**
 * Checkpoint type representing a block number and its associated digest.
 */
export interface Checkpoint {
    /** Block number */
    blockNumber: number;
    /** 0x-prefixed 32-byte hex digest */
    digest: string;
}

/**
 * Result of verifying a single checkpoint.
 */
export interface VerificationResult {
    /** Block number that was verified */
    blockNumber: number;
    /** Whether the checkpoint passed verification */
    passed: boolean;
    /** Expected digest from the CSV file */
    expected: string;
    /** Computed digest from block data */
    computed: string;
}

/**
 * Summary of verification results.
 */
export interface VerificationSummary {
    /** Total number of checkpoints verified */
    total: number;
    /** Number of checkpoints that passed */
    passed: number;
    /** Number of checkpoints that failed */
    failed: number;
    /** Individual verification results */
    results: VerificationResult[];
}
