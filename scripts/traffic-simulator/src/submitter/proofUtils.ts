/**
 * Proof utilities for fetching and submitting proofs
 *
 * Based on patterns from scripts/utils.js
 * Integrates with cc-next-query-builder for structured queries
 */

import { ethers, JsonRpcProvider } from 'ethers';
import type { FormattedProof, ProofResponse, QueryMode, TxInfo } from '../types.ts';
import { buildQuery, describeQueryMode, formatLayout } from '../query/queryFactory.ts';
import { debug } from '../logger.ts';

// Block prover precompile address
const PRECOMPILE_ADDRESS = '0x0000000000000000000000000000000000000FD2';
const DEFAULT_PROOF_API_TIMEOUT_MS = 30_000;
const DEFAULT_RPC_TIMEOUT_MS = 30_000;
const DEFAULT_RECEIPT_TIMEOUT_MS = 120_000;

type SignerEntry = {
  signer: ethers.NonceManager;
  provider: JsonRpcProvider;
};

const signerCache = new Map<string, SignerEntry>();
let submissionQueue: Promise<void> = Promise.resolve();

function getSignerKey(cc3HttpUrl: string, privateKey: string): string {
  return `${cc3HttpUrl}:${privateKey}`;
}

function getSigner(cc3HttpUrl: string, privateKey: string): SignerEntry {
  const key = getSignerKey(cc3HttpUrl, privateKey);
  const existing = signerCache.get(key);
  if (existing) {
    return existing;
  }

  const provider = new ethers.JsonRpcProvider(cc3HttpUrl);
  const wallet = new ethers.Wallet(privateKey, provider);
  const signer = new ethers.NonceManager(wallet);
  const entry = { signer, provider };
  signerCache.set(key, entry);
  return entry;
}

function resetSigner(cc3HttpUrl: string, privateKey: string): SignerEntry {
  const key = getSignerKey(cc3HttpUrl, privateKey);
  signerCache.delete(key);
  return getSigner(cc3HttpUrl, privateKey);
}

function isNonceError(error: unknown): boolean {
  if (!error || typeof error !== 'object') {
    return false;
  }
  const err = error as {
    code?: string;
    message?: string;
    shortMessage?: string;
    info?: { error?: { message?: string } };
  };
  const code = err.code ?? '';
  const message = `${err.message ?? ''} ${err.shortMessage ?? ''} ${err.info?.error?.message ?? ''}`.toLowerCase();
  return (
    code === 'NONCE_EXPIRED' ||
    message.includes('nonce too low') ||
    message.includes('nonce has already been used')
  );
}

export function isContinuityMismatchError(error: unknown): boolean {
  if (!error) {
    return false;
  }
  const message = error instanceof Error ? error.message : String(error);
  return message.includes('Continuity proof does not match attestation or checkpoint');
}

async function withNonceRetry<T>(
  cc3HttpUrl: string,
  privateKey: string,
  label: string,
  fn: (entry: SignerEntry) => Promise<T>,
): Promise<T> {
  try {
    return await fn(getSigner(cc3HttpUrl, privateKey));
  } catch (error) {
    if (isNonceError(error)) {
      console.warn(`⚠️  ${label}: nonce out of sync, refreshing signer and retrying...`);
      return await fn(resetSigner(cc3HttpUrl, privateKey));
    }
    throw error;
  }
}

async function withSubmissionLock<T>(fn: () => Promise<T>): Promise<T> {
  const previous = submissionQueue;
  let release: () => void;
  submissionQueue = new Promise<void>((resolve) => {
    release = resolve;
  });

  await previous;
  try {
    return await fn();
  } finally {
    release!();
  }
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | undefined;
  const startTime = Date.now();
  const timeoutPromise = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });

  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timeoutId !== undefined) {
      clearTimeout(timeoutId);
    }
    debug('Timing', { label, durationMs: Date.now() - startTime });
  }
}

// Block prover ABI (simplified for verifyAndEmit single + batch)
// Matches:
// - verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))
// - verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))
const PRECOMPILE_ABI = [
  {
    type: 'function',
    name: 'verifyAndEmit',
    inputs: [
      { name: 'chainKey', type: 'uint64' },
      { name: 'height', type: 'uint64' },
      { name: 'transaction', type: 'bytes' },
      {
        name: 'merkleProof',
        type: 'tuple',
        components: [
          { name: 'root', type: 'bytes32' },
          {
            name: 'siblings',
            type: 'tuple[]',
            components: [
              { name: 'hash', type: 'bytes32' },
              { name: 'isLeft', type: 'bool' },
            ],
          },
        ],
      },
      {
        name: 'continuityProof',
        type: 'tuple',
        components: [
          { name: 'lowerEndpointDigest', type: 'bytes32' },
          { name: 'roots', type: 'bytes32[]' },
        ],
      },
    ],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  {
    type: 'function',
    name: 'verifyAndEmit',
    inputs: [
      { name: 'chainKey', type: 'uint64' },
      { name: 'heights', type: 'uint64[]' },
      { name: 'transactions', type: 'bytes[]' },
      {
        name: 'merkleProofs',
        type: 'tuple[]',
        components: [
          { name: 'root', type: 'bytes32' },
          {
            name: 'siblings',
            type: 'tuple[]',
            components: [
              { name: 'hash', type: 'bytes32' },
              { name: 'isLeft', type: 'bool' },
            ],
          },
        ],
      },
      {
        name: 'continuityProof',
        type: 'tuple',
        components: [
          { name: 'lowerEndpointDigest', type: 'bytes32' },
          { name: 'roots', type: 'bytes32[]' },
        ],
      },
    ],
    outputs: [],
    stateMutability: 'nonpayable',
  },
  {
    type: 'event',
    name: 'TransactionVerified',
    inputs: [
      { name: 'chainKey', type: 'uint64', indexed: true },
      { name: 'height', type: 'uint64', indexed: true },
      { name: 'transactionIndex', type: 'uint64', indexed: false },
    ],
  },
  {
    type: 'error',
    name: 'ContinuityVerificationFailed',
    inputs: [],
  },
  {
    type: 'error',
    name: 'MerkleVerificationFailed',
    inputs: [],
  },
];

interface ProofApiErrorBody {
  code?: string;
  message?: string;
  retriable?: boolean;
}

class ProofApiError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly retriable?: boolean;

  constructor(message: string, status: number, code?: string, retriable?: boolean) {
    super(message);
    this.name = 'ProofApiError';
    this.status = status;
    this.code = code;
    this.retriable = retriable;
  }
}

async function fetchProofOnce(
  url: string,
  timeoutMs: number = DEFAULT_PROOF_API_TIMEOUT_MS,
): Promise<ProofResponse> {
  debug('Proof API request', { url, timeoutMs });
  const controller = new AbortController();
  const timeoutId: ReturnType<typeof setTimeout> = setTimeout(() => controller.abort(), timeoutMs);

  let response: Response;
  try {
    response = await fetch(url, { signal: controller.signal });
  } catch (error) {
    if (error instanceof DOMException && error.name === 'AbortError') {
      throw new Error(`Proof API request timed out after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timeoutId);
  }

  if (response.ok) {
    return (await response.json()) as ProofResponse;
  }

  const errorText = await response.text();
  let errorJson: ProofApiErrorBody | null = null;
  try {
    errorJson = JSON.parse(errorText) as ProofApiErrorBody;
  } catch {
    // Ignore JSON parse errors
  }

  const message = errorJson?.message ?? errorText ?? response.statusText;
  debug('Proof API error response', {
    url,
    status: response.status,
    statusText: response.statusText,
    code: errorJson?.code,
    retriable: errorJson?.retriable,
  });
  throw new ProofApiError(message, response.status, errorJson?.code, errorJson?.retriable);
}

async function fetchProofWithRetry(
  url: string,
  label: string,
  maxRetries = 5,
  initialDelay = 2000,
): Promise<ProofResponse> {
  let lastError: Error | null = null;

  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await fetchProofOnce(url);
    } catch (error) {
      debug('Proof API attempt failed', {
        label,
        attempt: attempt + 1,
        maxRetries,
        error: error instanceof Error ? error.message : String(error),
      });
      const isNetworkError =
        error instanceof Error &&
        (error.message.includes('fetch') ||
          error.message.includes('network') ||
          error.message.includes('timed out'));
      const isRetriableApiError =
        error instanceof ProofApiError &&
        (error.retriable === true || error.status === 500 || error.status === 503);

      if (attempt < maxRetries - 1 && (isNetworkError || isRetriableApiError)) {
        const delayMs = initialDelay * Math.pow(2, attempt);
        const jitter = Math.floor(Math.random() * 250);
        console.log(
          `⚠️  Proof API not ready (${label}, attempt ${attempt + 1}/${maxRetries}), waiting ${delayMs + jitter}ms...`,
        );
        await delay(delayMs + jitter);
        lastError = error instanceof Error ? error : new Error(String(error));
        continue;
      }

      throw error;
    }
  }

  if (lastError) {
    throw lastError;
  }

  throw new Error(`Failed to fetch proof after ${maxRetries} attempts`);
}

/**
 * Fetch proof from the proof generation API using transaction hash
 *
 * Uses the endpoint: GET /api/v1/proof-by-tx/{chain_key}/{tx_hash}
 */
export function fetchProofByTxHash(
  apiUrl: string,
  chainKey: number,
  txHash: string,
  maxRetries = 5,
  initialDelay = 2000,
): Promise<ProofResponse> {
  const url = `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${txHash}`;
  return fetchProofWithRetry(url, 'proof-by-tx', maxRetries, initialDelay);
}

/**
 * Fetch proof from the proof generation API using block + tx index
 *
 * Uses the endpoint: GET /api/v1/proof/{chain_key}/{header_number}/{tx_index}
 */
export function fetchProofByTxIndex(
  apiUrl: string,
  chainKey: number,
  headerNumber: number,
  txIndex: number,
  maxRetries = 5,
  initialDelay = 2000,
): Promise<ProofResponse> {
  const url = `${apiUrl}/api/v1/proof/${chainKey}/${headerNumber}/${txIndex}`;
  return fetchProofWithRetry(url, 'proof-by-index', maxRetries, initialDelay);
}

/**
 * Fetch proof using index when available, fall back to tx hash on index mismatch
 */
export async function fetchProofForTx(
  apiUrl: string,
  chainKey: number,
  txInfo: TxInfo,
  maxRetries = 5,
  initialDelay = 2000,
): Promise<ProofResponse> {
  debug('Fetching proof for tx', {
    chainKey,
    blockNumber: txInfo.blockNumber,
    txIndex: txInfo.txIndex,
    txHash: txInfo.txHash,
  });
  try {
    return await fetchProofByTxIndex(
      apiUrl,
      chainKey,
      txInfo.blockNumber,
      txInfo.txIndex,
      maxRetries,
      initialDelay,
    );
  } catch (error) {
    if (error instanceof ProofApiError && ['TxIndexOutOfBounds', 'InvalidParameter'].includes(error.code ?? '')) {
      console.warn(
        `⚠️  Proof-by-index failed (${error.code}), falling back to proof-by-tx for ${txInfo.txHash.slice(0, 10)}...`,
      );
      return await fetchProofByTxHash(apiUrl, chainKey, txInfo.txHash, maxRetries, initialDelay);
    }
    throw error;
  }
}

/**
 * Convert API proof format to precompile format
 * 
 * The API returns camelCase field names directly, so this is mostly a passthrough
 * with validation.
 */
export function convertProofFormat(apiProof: ProofResponse): FormattedProof {
  if (!apiProof.merkleProof) {
    throw new MerkleProofMissingError('Merkle proof is missing from API response');
  }

  if (!apiProof.continuityProof) {
    throw new Error('Continuity proof is missing from API response');
  }

  return {
    continuityProof: {
      lowerEndpointDigest: apiProof.continuityProof.lowerEndpointDigest,
      roots: apiProof.continuityProof.roots,
    },
    merkleProof: {
      root: apiProof.merkleProof.root,
      siblings: apiProof.merkleProof.siblings.map((s) => ({
        hash: s.hash,
        isLeft: s.isLeft,
      })),
    },
  };
}

/**
 * Custom error for missing merkle proof (retriable)
 */
export class MerkleProofMissingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'MerkleProofMissingError';
  }
}

/**
 * Submit proof to the block prover precompile
 */
export async function submitToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  blockHeight: number,
  txBytes: Uint8Array,
  proof: FormattedProof,
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  return await withSubmissionLock(() =>
    withNonceRetry(cc3HttpUrl, privateKey, 'Single submit', async ({ signer, provider }) => {
      debug('Submitting proof', {
        chainKey,
        blockHeight,
        txBytesLength: txBytes.length,
        merkleSiblings: proof.merkleProof.siblings.length,
        continuityRoots: proof.continuityProof.roots.length,
        cc3HttpUrl,
      });
      // Create contract instance
      const precompile = new ethers.Contract(PRECOMPILE_ADDRESS, PRECOMPILE_ABI, signer);
      const iface = precompile.interface;

      // Prepare proof tuples
      const merkleProofTuple = [
        proof.merkleProof.root,
        proof.merkleProof.siblings.map((s) => [s.hash, s.isLeft]),
      ];

      const continuityProofTuple = [
        proof.continuityProof.lowerEndpointDigest,
        proof.continuityProof.roots,
      ];

      // Encode transaction data
      const txBytesHex = ethers.hexlify(txBytes);

      // Get function fragment
      // Matches: verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))
      const funcFragment = iface.getFunction(
        'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))',
      );

      if (!funcFragment) {
        throw new Error('verifyAndEmit function not found in ABI');
      }

      const params = [BigInt(chainKey), BigInt(blockHeight), txBytesHex, merkleProofTuple, continuityProofTuple];
      const data = iface.encodeFunctionData(funcFragment, params);

      // Simulate call first
      const signerAddress = await signer.getAddress();
      try {
        await withTimeout(
          provider.call({ to: PRECOMPILE_ADDRESS, data, from: signerAddress }),
          DEFAULT_RPC_TIMEOUT_MS,
          'Precompile simulation',
        );
      } catch (simError) {
        const error = simError as Error & { data?: string; error?: { data?: string } };
        const revertData = error.data || error.error?.data;
        if (revertData) {
          try {
            const parsed = iface.parseError(revertData);
            throw new Error(`Transaction will revert: ${parsed?.name ?? 'Unknown error'}`);
          } catch {
            // Fall through
          }
        }
        throw new Error(`Transaction simulation failed: ${error.message}`);
      }

      // Send transaction
      const tx = await signer.sendTransaction({
        to: PRECOMPILE_ADDRESS,
        data,
        gasLimit: 5_000_000n,
      });
      debug('Single tx sent', { txHash: tx.hash, nonce: tx.nonce });

      const receipt = await withTimeout(
        tx.wait(),
        DEFAULT_RECEIPT_TIMEOUT_MS,
        'Transaction confirmation',
      );

      if (!receipt || receipt.status !== 1) {
        throw new Error('Transaction reverted');
      }
      debug('Single tx confirmed', {
        txHash: receipt.hash,
        status: receipt.status,
        gasUsed: receipt.gasUsed,
      });

      return {
        success: true,
        txHash: receipt.hash,
        gasUsed: receipt.gasUsed,
      };
    })
  );
}

/**
 * Submit batch proofs to the block prover precompile
 */
export async function submitBatchToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  heights: number[],
  txBytesList: Uint8Array[],
  merkleProofs: FormattedProof['merkleProof'][],
  continuityProof: FormattedProof['continuityProof'],
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  return await withSubmissionLock(() =>
    withNonceRetry(cc3HttpUrl, privateKey, 'Batch submit', async ({ signer, provider }) => {
      debug('Submitting batch', {
        chainKey,
        batchSize: heights.length,
        continuityRoots: continuityProof.roots.length,
        cc3HttpUrl,
      });
      const precompile = new ethers.Contract(PRECOMPILE_ADDRESS, PRECOMPILE_ABI, signer);
      const iface = precompile.interface;

      const merkleProofTuples = merkleProofs.map((proof) => [
        proof.root,
        proof.siblings.map((s) => [s.hash, s.isLeft]),
      ]);
      const continuityProofTuple = [
        continuityProof.lowerEndpointDigest,
        continuityProof.roots,
      ];
      const txBytesHexes = txBytesList.map((txBytes) => ethers.hexlify(txBytes));
      const heightsU64 = heights.map((height) => BigInt(height));

      const funcFragment = iface.getFunction(
        'verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))',
      );

      if (!funcFragment) {
        throw new Error('verifyAndEmit batch function not found in ABI');
      }

      const params = [BigInt(chainKey), heightsU64, txBytesHexes, merkleProofTuples, continuityProofTuple];
      const data = iface.encodeFunctionData(funcFragment, params);

      // Simulate call first
      const signerAddress = await signer.getAddress();
      try {
        await withTimeout(
          provider.call({ to: PRECOMPILE_ADDRESS, data, from: signerAddress }),
          DEFAULT_RPC_TIMEOUT_MS,
          'Batch precompile simulation',
        );
      } catch (simError) {
        const error = simError as Error & { data?: string; error?: { data?: string } };
        const revertData = error.data || error.error?.data;
        if (revertData) {
          try {
            const parsed = iface.parseError(revertData);
            throw new Error(`Batch transaction will revert: ${parsed?.name ?? 'Unknown error'}`);
          } catch {
            // Fall through
          }
        }
        throw new Error(`Batch transaction simulation failed: ${error.message}`);
      }

      const tx = await signer.sendTransaction({
        to: PRECOMPILE_ADDRESS,
        data,
        gasLimit: 10_000_000n,
      });
      debug('Batch tx sent', { txHash: tx.hash, nonce: tx.nonce });

      const receipt = await withTimeout(
        tx.wait(),
        DEFAULT_RECEIPT_TIMEOUT_MS,
        'Batch transaction confirmation',
      );

      if (!receipt || receipt.status !== 1) {
        throw new Error('Batch transaction reverted');
      }
      debug('Batch tx confirmed', {
        txHash: receipt.hash,
        status: receipt.status,
        gasUsed: receipt.gasUsed,
      });

      return {
        success: true,
        txHash: receipt.hash,
        gasUsed: receipt.gasUsed,
      };
    })
  );
}

/**
 * Convert WebSocket URL to HTTP URL for JSON-RPC calls
 * Handles Infura's different path structure (wss://.../ws/v3/KEY -> https://.../v3/KEY)
 */
function wsToHttpUrl(wsUrl: string): string {
  let httpUrl = wsUrl
    // Convert protocol
    .replace(/^wss:\/\//, 'https://')
    .replace(/^ws:\/\//, 'http://');
  
  // Handle Infura's /ws/ path segment
  // wss://sepolia.infura.io/ws/v3/KEY -> https://sepolia.infura.io/v3/KEY
  if (httpUrl.includes('.infura.io/ws/')) {
    httpUrl = httpUrl.replace('/ws/', '/');
  }
  
  // Handle Alchemy's wss subdomain pattern (already works with https)
  // wss://eth-sepolia.g.alchemy.com/v2/KEY -> https://eth-sepolia.g.alchemy.com/v2/KEY
  
  return httpUrl;
}

/**
 * Delay helper
 */
function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Fetch and submit a proof using optional query builder logging.
 *
 * When enabled, the query builder constructs a layout for logging/debugging.
 * Proof retrieval uses the index-based endpoint by default (falls back to tx hash).
 */
export async function fetchAndSubmitProofWithQueryBuilder(
  sourceRpcUrl: string,
  proofApiUrl: string,
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  txInfo: TxInfo,
  queryMode: QueryMode,
  enableQueryBuilder = true,
  maxRetries = 5,
  retryDelayMs = 2000,
): Promise<{ success: boolean; txHash: string; gasUsed: bigint; queryMode: string }> {
  let sourceProvider: JsonRpcProvider | null = null;

  try {
    if (enableQueryBuilder) {
      const httpUrl = wsToHttpUrl(sourceRpcUrl);
      sourceProvider = new JsonRpcProvider(httpUrl);

      console.log(`   📋 Building query with mode: ${describeQueryMode(queryMode)}`);
      const queryResult = await buildQuery(sourceProvider, txInfo.txHash, queryMode);
      console.log(`   ✅ Query built for block ${queryResult.blockNumber}, tx index ${queryResult.txIndex}`);
      console.log(`   📐 Layout segments: ${formatLayout(queryResult.layout)}`);
    }

    const apiProof = await fetchProofForTx(
      proofApiUrl,
      chainKey,
      txInfo,
      maxRetries,
      retryDelayMs,
    );
    const proof = convertProofFormat(apiProof);

    if (apiProof.headerNumber !== txInfo.blockNumber) {
      console.warn(
        `⚠️  Proof header mismatch: expected ${txInfo.blockNumber}, got ${apiProof.headerNumber}`,
      );
    }

    // Transaction bytes come from the proof API
    if (!apiProof.txBytes) {
      throw new Error('Transaction bytes not found in API response');
    }
    const txBytes = ethers.getBytes(apiProof.txBytes);

    // Submit to precompile
    const result = await submitToPrecompile(
      cc3HttpUrl,
      privateKey,
      chainKey,
      apiProof.headerNumber,
      txBytes,
      proof,
    );

    return {
      ...result,
      queryMode: queryMode,
    };
  } finally {
    // Clean up provider to prevent background retries
    sourceProvider?.destroy();
  }
}
