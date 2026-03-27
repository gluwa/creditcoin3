/**
 * Type definitions for the proof traffic simulator
 */

/**
 * Information about a block from the source chain
 */
export interface BlockInfo {
  /** Block number */
  blockNumber: number;
  /** Transaction hashes in this block */
  txHashes: string[];
  /** Timestamp when received/queued */
  timestamp: number;
}

/**
 * Simulator configuration
 */
export interface SimulatorConfig {
  /** WebSocket RPC URL for source chain */
  sourceRpcUrl: string;
  /** Chain key for the source chain */
  chainKey: number;

  // Creditcoin3
  /** WebSocket URL for CC3 */
  cc3WsUrl: string;
  /** HTTP URL for CC3 */
  cc3HttpUrl: string;
  /** Private key for signing submissions */
  cc3PrivateKey: string;

  // Proof API
  /** Proof generation API URL */
  proofApiUrl: string;

  // Simulation parameters
  /** Maximum blocks to track in queue */
  maxQueueSize: number;
  /** Number of transactions per batch submission */
  batchSize: number;
  /** Probability of using batch mode (0.0 - 1.0) */
  batchProbability: number;
  /** Submit a single proof once every N blocks */
  singleEveryBlocks: number;

  // Server
  /** Port for health check server */
  healthPort: number;
}

/**
 * Health status of the simulator
 */
export interface HealthStatus {
  /** Whether connected to source chain */
  sourceChainConnected: boolean;
  /** Whether connected to CC3 */
  cc3Connected: boolean;
  /** Source chain key */
  sourceChainKey: number;
  /** CC3 WebSocket URL */
  cc3WsUrl: string;
  /** Current queue size */
  queueSize: number;
  /** Total blocks processed */
  blocksProcessed: number;
  /** Total proofs submitted */
  proofsSubmitted: number;
  /** Total single submissions */
  singleSubmissions: number;
  /** Total batch submissions */
  batchSubmissions: number;
  /** Total proof submission errors */
  proofErrors: number;
  /** Last error message if any */
  lastError: string | null;
  /** Unique errors with occurrence counts */
  uniqueErrors: Record<string, number>;
  /** Uptime in seconds */
  uptimeSeconds: number;
}

/**
 * Proof response from the proof generation API (camelCase format)
 *
 * Example response from: https://proof-gen-api.usc-testnet2.creditcoin.network/api/v1/proof-by-tx/...
 */
export interface ProofResponse {
  cached: boolean;
  chainKey: number;
  headerNumber: number;
  txIndex?: number;
  txHash?: string;
  txBytes?: string;
  continuityProof: {
    lowerEndpointDigest: string;
    /** Array of merkle roots for the continuity chain */
    roots: string[];
  };
  merkleProof?: {
    root: string;
    siblings: Array<{
      hash: string;
      isLeft: boolean;
    }>;
  };
  generatedAt?: string;
}

/**
 * Batch proof query for the proof-gen-api batch endpoint
 */
export interface ProofQuery {
  headerNumber: number;
  txIndexes: number[];
}

/**
 * Merkle proof entry in a batch response
 */
export interface BatchMerkleProofEntry {
  txHash?: string;
  txBytes?: string;
  merkleProof: {
    root: string;
    siblings: Array<{ hash: string; isLeft: boolean }>;
  };
}

/**
 * Response from the proof-gen-api batch endpoint
 * POST /api/v1/proof-batch/{chain_key}
 */
export interface BatchProofResponse {
  chainKey: number;
  fromHeader: number;
  toHeader: number;
  continuityProof: {
    lowerEndpointDigest: string;
    roots: string[];
  };
  /** Nested map: blockNumber -> txIndex -> proof entry */
  merkleProofs: Record<string, Record<string, BatchMerkleProofEntry>>;
  cached: boolean;
  generatedAt?: string;
}

/**
 * Formatted proof for precompile submission
 */
export interface FormattedProof {
  continuityProof: {
    lowerEndpointDigest: string;
    /** Array of merkle roots */
    roots: string[];
  };
  merkleProof: {
    root: string;
    siblings: Array<{
      hash: string;
      isLeft: boolean;
    }>;
  };
}

/**
 * Transaction info for submission
 */
export interface TxInfo {
  txHash: string;
  blockNumber: number;
  /** Transaction index within the block */
  txIndex: number;
}

/**
 * Metrics for Prometheus export
 */
export interface Metrics {
  blocksQueued: number;
  blocksProcessed: number;
  proofsSubmitted: number;
  singleSubmissions: number;
  batchSubmissions: number;
  proofErrors: number;
  queueSize: number;
  sourceChainConnected: number;
  cc3Connected: number;
}
