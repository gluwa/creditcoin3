/**
 * Proof utilities for fetching and submitting proofs
 */

import { ethers, JsonRpcProvider } from "ethers";
import type {
  BatchProofResponse,
  FormattedProof,
  ProofQuery,
  ProofResponse,
  TxInfo,
} from "../types.ts";
import { debug } from "../logger.ts";
import {
  BATCH_PROOF_GAS_LIMIT,
  BATCH_VERIFY_SIG,
  MAX_TRANSIENT_RETRIES,
  MIN_PRIORITY_FEE_GWEI,
  PRECOMPILE_ADDRESS,
  PROOF_API_BASE_DELAY_MS,
  PROOF_API_MAX_RETRIES,
  PROOF_API_TIMEOUT_MS,
  RECEIPT_TIMEOUT_MS,
  RPC_TIMEOUT_MS,
  SINGLE_PROOF_GAS_LIMIT,
  SINGLE_VERIFY_SIG,
  TRANSIENT_RETRY_BASE_DELAY_MS,
} from "../constants.ts";
import { sleep } from "../utils/reconnect.ts";
import { withTimeout } from "../utils/retry.ts";
import PRECOMPILE_ABI from "../abi/block_prover.json" with {
  type: "json",
};

// ============================================================================
// Signer Management
// ============================================================================

type SignerEntry = { signer: ethers.NonceManager; provider: JsonRpcProvider };
const signerCache = new Map<string, SignerEntry>();
let submissionQueue: Promise<void> = Promise.resolve();

function getSigner(cc3HttpUrl: string, privateKey: string): SignerEntry {
  const key = `${cc3HttpUrl}:${privateKey}`;
  let entry = signerCache.get(key);
  if (!entry) {
    const provider = new ethers.JsonRpcProvider(cc3HttpUrl);
    const wallet = new ethers.Wallet(privateKey, provider);
    entry = { signer: new ethers.NonceManager(wallet), provider };
    signerCache.set(key, entry);
  }
  return entry;
}

function resetSigner(cc3HttpUrl: string, privateKey: string): SignerEntry {
  signerCache.delete(`${cc3HttpUrl}:${privateKey}`);
  return getSigner(cc3HttpUrl, privateKey);
}

// ============================================================================
// Error Detection
// ============================================================================

function isNonceError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  return msg.includes("nonce too low") ||
    msg.includes("nonce has already been used");
}

function isReplacementUnderpricedError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  return msg.includes("replacement transaction underpriced");
}

export function isContinuityMismatchError(error: unknown): boolean {
  return getErrorMessage(error).includes("Continuity proof does not match");
}

/**
 * Detect transient network errors that should trigger a retry with provider reset
 */
function isTransientNetworkError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  return (
    msg.includes("socket hang up") ||
    msg.includes("econnreset") ||
    msg.includes("econnrefused") ||
    msg.includes("etimedout") ||
    msg.includes("enotfound") ||
    msg.includes("network error") ||
    msg.includes("fetch failed") ||
    msg.includes("connection reset") ||
    msg.includes("connection refused") ||
    msg.includes("connection error") ||
    msg.includes("socket disconnected") ||
    (msg.includes("request to") && msg.includes("failed"))
  );
}

function getErrorMessage(error: unknown): string {
  if (!error) return "";
  if (error instanceof Error) return error.message;
  if (typeof error === "object") {
    const e = error as Record<string, unknown>;
    return `${e.message ?? ""} ${e.shortMessage ?? ""}`;
  }
  return String(error);
}

/** Error(string) selector: 0x08c379a0. Precompile uses this for revert messages. */
const ERROR_STRING_SELECTOR = "0x08c379a0";

function decodeRevertMessage(revertData: string): string {
  if (typeof revertData !== "string" || !revertData.startsWith("0x")) {
    return "Unknown";
  }
  // Precompile uses Error(string) - decode the message
  if (revertData.startsWith(ERROR_STRING_SELECTOR) && revertData.length > 10) {
    try {
      const decoded = ethers.AbiCoder.defaultAbiCoder().decode(
        ["string"],
        "0x" + revertData.slice(10),
      );
      return decoded[0] as string;
    } catch {
      return "Error(string) decode failed";
    }
  }
  // Try ABI parse for custom errors
  const iface = new ethers.Interface(PRECOMPILE_ABI);
  const parsed = iface.parseError(revertData);
  if (parsed) {
    const args = parsed.args ? ` (${parsed.args.join(", ")})` : "";
    return `${parsed.name}${args}`;
  }
  return "Unknown";
}

// ============================================================================
// Utilities
// ============================================================================

async function withSubmissionLock<T>(fn: () => Promise<T>): Promise<T> {
  const previous = submissionQueue;
  let release: () => void;
  submissionQueue = new Promise((r) => (release = r));
  await previous;
  try {
    return await fn();
  } finally {
    release!();
  }
}

// ============================================================================
// Fee Management
// ============================================================================

async function getFeeOverrides(
  provider: JsonRpcProvider,
): Promise<ethers.TransactionRequest> {
  const feeData = await withTimeout(
    provider.getFeeData(),
    RPC_TIMEOUT_MS,
    "Fee data",
  );
  const minPriority = MIN_PRIORITY_FEE_GWEI * 1_000_000_000n;

  if (feeData.maxFeePerGas !== null) {
    const priority = feeData.maxPriorityFeePerGas ?? minPriority;
    return {
      maxFeePerGas: feeData.maxFeePerGas,
      maxPriorityFeePerGas: priority < minPriority ? minPriority : priority,
    };
  }

  return { gasPrice: feeData.gasPrice ?? minPriority };
}

function bumpFees(
  overrides: ethers.TransactionRequest,
): ethers.TransactionRequest {
  const bump = (
    v: bigint | null | undefined,
  ) => (v ? (v * 120n) / 100n + 1n : undefined);
  return {
    maxFeePerGas: bump(overrides.maxFeePerGas as bigint),
    maxPriorityFeePerGas: bump(overrides.maxPriorityFeePerGas as bigint),
    gasPrice: bump(overrides.gasPrice as bigint),
  };
}

// ============================================================================
// Transaction Submission
// ============================================================================

async function sendTransaction(
  signer: ethers.NonceManager,
  provider: JsonRpcProvider,
  request: ethers.TransactionRequest,
  label: string,
): Promise<ethers.TransactionResponse> {
  try {
    return await signer.sendTransaction(request);
  } catch (error) {
    if (!isReplacementUnderpricedError(error)) throw error;

    const address = await signer.getAddress();
    const nonce = await withTimeout(
      provider.getTransactionCount(address, "pending"),
      RPC_TIMEOUT_MS,
      `${label} nonce`,
    );
    debug("Retrying underpriced tx", { label, nonce });
    return await signer.sendTransaction({
      ...request,
      nonce,
      ...bumpFees(request),
    });
  }
}

async function waitForReceipt(
  provider: JsonRpcProvider,
  tx: ethers.TransactionResponse,
  label: string,
): Promise<ethers.TransactionReceipt> {
  try {
    const receipt = await withTimeout(tx.wait(), RECEIPT_TIMEOUT_MS, label);
    if (!receipt) throw new Error(`${label} returned empty receipt`);
    return receipt;
  } catch (error) {
    // Handle replaced transactions
    const err = error as { code?: string; receipt?: ethers.TransactionReceipt };
    if (err.code === "TRANSACTION_REPLACED" && err.receipt) {
      debug("Transaction replaced", {
        label,
        original: tx.hash,
        replacement: err.receipt.hash,
      });
      return err.receipt;
    }

    // Try direct lookup as fallback
    const receipt = await provider.getTransactionReceipt(tx.hash).catch(() =>
      null
    );
    if (receipt) return receipt;

    throw error;
  }
}

// ============================================================================
// Precompile Submission (Unified)
// ============================================================================

interface PrecompileParams {
  cc3HttpUrl: string;
  privateKey: string;
  chainKey: number;
  data: string;
  gasLimit: bigint;
  label: string;
}

async function submitToPrecompileInternal(
  params: PrecompileParams,
): Promise<{ txHash: string; gasUsed: bigint }> {
  const { cc3HttpUrl, privateKey, data, gasLimit, label } = params;

  /**
   * Simulate the transaction with retry for transient network errors.
   * This is safe to retry because no nonce is used during simulation.
   * Returns the entry (which may be refreshed) along with the fee overrides.
   */
  const simulateWithRetry = async (
    initialEntry: SignerEntry,
  ): Promise<
    { entry: SignerEntry; feeOverrides: ethers.TransactionRequest }
  > => {
    let entry = initialEntry;

    for (let attempt = 0; attempt <= MAX_TRANSIENT_RETRIES; attempt++) {
      // Destructure inside the loop to use fresh references after reset
      const { signer, provider } = entry;

      try {
        const from = await signer.getAddress();

        // Simulate the transaction
        await withTimeout(
          provider.call({ to: PRECOMPILE_ADDRESS, data, from }),
          RPC_TIMEOUT_MS,
          `${label} simulation`,
        );

        // Get fee data
        const feeOverrides = await getFeeOverrides(provider);
        debug(`${label} fees`, { ...feeOverrides });

        return { entry, feeOverrides };
      } catch (error) {
        // Check for revert errors (not retriable)
        const revertData = (error as { data?: string }).data;
        if (revertData) {
          const revertMsg = decodeRevertMessage(revertData);
          throw new Error(`${label} will revert: ${revertMsg}`);
        }

        // Check if it's a transient network error that should be retried
        if (isTransientNetworkError(error) && attempt < MAX_TRANSIENT_RETRIES) {
          const delayMs = TRANSIENT_RETRY_BASE_DELAY_MS * Math.pow(2, attempt) +
            Math.random() * 500;
          console.warn(
            `⚠️  ${label}: transient network error during simulation (${
              getErrorMessage(error)
            }), retrying in ${Math.round(delayMs / 1000)}s (${
              attempt + 1
            }/${MAX_TRANSIENT_RETRIES})...`,
          );
          // Reset the provider and update entry for next iteration
          entry = resetSigner(cc3HttpUrl, privateKey);
          await sleep(delayMs);
          continue;
        }

        throw new Error(
          `${label} simulation failed: ${getErrorMessage(error)}`,
        );
      }
    }

    // Should never reach here
    throw new Error(
      `${label} simulation failed after ${MAX_TRANSIENT_RETRIES} retries`,
    );
  };

  /**
   * Execute the full submission (simulate + send).
   * Transient retries only happen during simulation, not after tx is sent.
   */
  const execute = async (initialEntry: SignerEntry) => {
    // Simulation phase with transient retry (safe - no nonce used)
    // Use the returned entry in case it was refreshed during retry
    const { entry, feeOverrides } = await simulateWithRetry(initialEntry);
    const { signer, provider } = entry;

    // Send transaction phase (no transient retry - nonce is used)
    // If this fails, we rely on nonce error handling, not blind retry
    const tx = await sendTransaction(
      signer,
      provider,
      { to: PRECOMPILE_ADDRESS, data, gasLimit, ...feeOverrides },
      label,
    );
    debug(`${label} sent`, { txHash: tx.hash, nonce: tx.nonce });

    return { tx, provider };
  };

  // Execute with submission lock and nonce retry
  const { tx, provider } = await withSubmissionLock(async () => {
    try {
      return await execute(getSigner(cc3HttpUrl, privateKey));
    } catch (error) {
      if (isNonceError(error)) {
        console.warn(`⚠️  ${label}: nonce out of sync, retrying...`);
        return await execute(resetSigner(cc3HttpUrl, privateKey));
      }
      throw error;
    }
  });

  const receipt = await waitForReceipt(provider, tx, `${label} confirmation`);
  if (receipt.status !== 1) throw new Error(`${label} reverted`);

  debug(`${label} confirmed`, {
    txHash: receipt.hash,
    gasUsed: receipt.gasUsed,
  });
  return { txHash: receipt.hash, gasUsed: receipt.gasUsed };
}

// ============================================================================
// Proof Encoding
// ============================================================================

function encodeMerkleProof(proof: FormattedProof["merkleProof"]) {
  return [proof.root, proof.siblings.map((s) => [s.hash, s.isLeft])];
}

function encodeContinuityProof(proof: FormattedProof["continuityProof"]) {
  return [proof.lowerEndpointDigest, proof.roots];
}

// ============================================================================
// Public Submission Functions
// ============================================================================

export async function submitToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  blockHeight: number,
  txBytes: Uint8Array,
  proof: FormattedProof,
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  const iface = new ethers.Interface(PRECOMPILE_ABI);
  const func = iface.getFunction(SINGLE_VERIFY_SIG);
  if (!func) throw new Error("Single verifyAndEmit not found in ABI");

  const data = iface.encodeFunctionData(func, [
    BigInt(chainKey),
    BigInt(blockHeight),
    ethers.hexlify(txBytes),
    encodeMerkleProof(proof.merkleProof),
    encodeContinuityProof(proof.continuityProof),
  ]);

  debug("Submitting single proof", {
    chainKey,
    blockHeight,
    txBytesLen: txBytes.length,
  });

  const result = await submitToPrecompileInternal({
    cc3HttpUrl,
    privateKey,
    chainKey,
    data,
    gasLimit: SINGLE_PROOF_GAS_LIMIT,
    label: "Single submit",
  });

  return { success: true, ...result };
}

export async function submitBatchToPrecompile(
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  heights: number[],
  txBytesList: Uint8Array[],
  merkleProofs: FormattedProof["merkleProof"][],
  continuityProof: FormattedProof["continuityProof"],
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  const iface = new ethers.Interface(PRECOMPILE_ABI);
  const func = iface.getFunction(BATCH_VERIFY_SIG);
  if (!func) throw new Error("Batch verifyAndEmit not found in ABI");

  const data = iface.encodeFunctionData(func, [
    BigInt(chainKey),
    heights.map(BigInt),
    txBytesList.map((b) => ethers.hexlify(b)),
    merkleProofs.map(encodeMerkleProof),
    encodeContinuityProof(continuityProof),
  ]);

  debug("Submitting batch", { chainKey, batchSize: heights.length });

  const result = await submitToPrecompileInternal({
    cc3HttpUrl,
    privateKey,
    chainKey,
    data,
    gasLimit: BATCH_PROOF_GAS_LIMIT,
    label: "Batch submit",
  });

  return { success: true, ...result };
}

// ============================================================================
// Proof API Client
// ============================================================================

class ProofApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly retriable?: boolean,
  ) {
    super(message);
    this.name = "ProofApiError";
  }
}

async function fetchProof(url: string): Promise<ProofResponse> {
  debug("Proof API request", { url });

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), PROOF_API_TIMEOUT_MS);

  try {
    const response = await fetch(url, { signal: controller.signal });
    clearTimeout(timeoutId);

    if (response.ok) {
      return (await response.json()) as ProofResponse;
    }

    const text = await response.text();
    let parsed: { code?: string; message?: string; retriable?: boolean } = {};
    try {
      parsed = JSON.parse(text);
    } catch { /* ignore */ }

    throw new ProofApiError(
      parsed.message || text || `${response.status} ${response.statusText}`,
      response.status,
      parsed.code,
      parsed.retriable,
    );
  } catch (error) {
    clearTimeout(timeoutId);
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error(`Proof API timed out after ${PROOF_API_TIMEOUT_MS}ms`);
    }
    throw error;
  }
}

interface FetchProofContext {
  blockNumber: number;
  /** Transaction index (for proof-by-index) or hash (for proof-by-tx) */
  txIdentifier: number | string;
}

function isProofFetchRetriable(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  // Timeout
  if (msg.includes("timed out")) return true;
  // Network errors (like SubmitBatchProof.js)
  if (
    msg.includes("fetch") ||
    msg.includes("network") ||
    msg.includes("econnreset") ||
    msg.includes("econnrefused") ||
    msg.includes("etimedout") ||
    msg.includes("enotfound") ||
    msg.includes("socket hang up") ||
    msg.includes("connection reset") ||
    msg.includes("connection refused")
  ) {
    return true;
  }
  // API 500 (e.g. "No attestation or checkpoint found after")
  if (
    error instanceof ProofApiError && (error.retriable || error.status >= 500)
  ) {
    return true;
  }
  return false;
}

async function fetchProofWithRetry(
  url: string,
  label: string,
  context: FetchProofContext,
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<ProofResponse> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await fetchProof(url);
    } catch (error) {
      if (attempt < maxRetries - 1 && isProofFetchRetriable(error)) {
        const delayMs = PROOF_API_BASE_DELAY_MS * Math.pow(2, attempt) +
          Math.random() * 250;

        const txDisplay = typeof context.txIdentifier === "number"
          ? `txIndex=${context.txIdentifier}`
          : `txHash=${String(context.txIdentifier).slice(0, 10)}...`;

        console.log(
          `⚠️  Proof API retry (${label}, ${
            attempt + 1
          }/${maxRetries})... blockNumber=${context.blockNumber}, ${txDisplay}`,
        );
        await sleep(delayMs);
        continue;
      }
      throw error;
    }
  }
  throw new Error(`Failed to fetch proof after ${maxRetries} attempts`);
}

export async function fetchProofForTx(
  apiUrl: string,
  chainKey: number,
  txInfo: TxInfo,
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<ProofResponse> {
  debug("Fetching proof", {
    chainKey,
    block: txInfo.blockNumber,
    txIndex: txInfo.txIndex,
  });

  const indexUrl =
    `${apiUrl}/api/v1/proof/${chainKey}/${txInfo.blockNumber}/${txInfo.txIndex}`;
  const context: FetchProofContext = {
    blockNumber: txInfo.blockNumber,
    txIdentifier: txInfo.txIndex,
  };

  try {
    return await fetchProofWithRetry(
      indexUrl,
      "proof-by-index",
      context,
      maxRetries,
    );
  } catch (error) {
    // Fall back to tx hash lookup on index errors
    if (
      error instanceof ProofApiError &&
      ["TxIndexOutOfBounds", "InvalidParameter"].includes(error.code ?? "")
    ) {
      console.warn(
        `⚠️  Falling back to proof-by-tx for ${txInfo.txHash.slice(0, 10)}...`,
      );
      const hashUrl =
        `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${txInfo.txHash}`;
      const hashContext: FetchProofContext = {
        blockNumber: txInfo.blockNumber,
        txIdentifier: txInfo.txHash,
      };
      return await fetchProofWithRetry(
        hashUrl,
        "proof-by-tx",
        hashContext,
        maxRetries,
      );
    }
    throw error;
  }
}

// ============================================================================
// Batch Proof API Client
// ============================================================================

async function fetchBatchProof(
  url: string,
  queries: ProofQuery[],
): Promise<BatchProofResponse> {
  debug("Batch Proof API request", { url, queries: queries.length });

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), PROOF_API_TIMEOUT_MS);

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(queries),
      signal: controller.signal,
    });
    clearTimeout(timeoutId);

    if (response.ok) {
      return (await response.json()) as BatchProofResponse;
    }

    const text = await response.text();
    let parsed: { code?: string; message?: string; retriable?: boolean } = {};
    try {
      parsed = JSON.parse(text);
    } catch { /* ignore */ }

    throw new ProofApiError(
      parsed.message || text || `${response.status} ${response.statusText}`,
      response.status,
      parsed.code,
      parsed.retriable,
    );
  } catch (error) {
    clearTimeout(timeoutId);
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error(
        `Batch Proof API timed out after ${PROOF_API_TIMEOUT_MS}ms`,
      );
    }
    throw error;
  }
}

export async function fetchBatchProofs(
  apiUrl: string,
  chainKey: number,
  queries: ProofQuery[],
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<BatchProofResponse> {
  const url = `${apiUrl}/api/v1/proof-batch/${chainKey}`;
  const context: FetchProofContext = {
    blockNumber: queries[0]?.headerNumber ?? 0,
    txIdentifier: `batch(${queries.length} queries)`,
  };

  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await fetchBatchProof(url, queries);
    } catch (error) {
      if (attempt < maxRetries - 1 && isProofFetchRetriable(error)) {
        const delayMs = PROOF_API_BASE_DELAY_MS * Math.pow(2, attempt) +
          Math.random() * 250;

        console.log(
          `⚠️  Batch Proof API retry (${
            attempt + 1
          }/${maxRetries})... ${context.txIdentifier}`,
        );
        await sleep(delayMs);
        continue;
      }
      throw error;
    }
  }
  throw new Error(
    `Failed to fetch batch proof after ${maxRetries} attempts`,
  );
}

// ============================================================================
// Proof Conversion
// ============================================================================

export class MerkleProofMissingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MerkleProofMissingError";
  }
}

export function convertProofFormat(apiProof: ProofResponse): FormattedProof {
  if (!apiProof.merkleProof) {
    throw new MerkleProofMissingError("Merkle proof missing from API response");
  }
  if (!apiProof.continuityProof) {
    throw new Error("Continuity proof missing from API response");
  }

  return {
    continuityProof: apiProof.continuityProof,
    merkleProof: {
      root: apiProof.merkleProof.root,
      siblings: apiProof.merkleProof.siblings.map((s) => ({
        hash: s.hash,
        isLeft: s.isLeft,
      })),
    },
  };
}

// ============================================================================
// High-Level API
// ============================================================================

export async function fetchAndSubmitProof(
  proofApiUrl: string,
  cc3HttpUrl: string,
  privateKey: string,
  chainKey: number,
  txInfo: TxInfo,
): Promise<{ success: boolean; txHash: string; gasUsed: bigint }> {
  const apiProof = await fetchProofForTx(proofApiUrl, chainKey, txInfo);
  const proof = convertProofFormat(apiProof);

  if (apiProof.headerNumber !== txInfo.blockNumber) {
    console.warn(
      `⚠️  Header mismatch: expected ${txInfo.blockNumber}, got ${apiProof.headerNumber}`,
    );
  }

  if (!apiProof.txBytes) {
    throw new Error("Transaction bytes not found in API response");
  }

  return submitToPrecompile(
    cc3HttpUrl,
    privateKey,
    chainKey,
    apiProof.headerNumber,
    ethers.getBytes(apiProof.txBytes),
    proof,
  );
}
