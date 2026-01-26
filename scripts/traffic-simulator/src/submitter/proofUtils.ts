/**
 * Proof utilities for fetching and submitting proofs
 */

import { ethers, JsonRpcProvider } from "ethers";
import type { FormattedProof, ProofResponse, TxInfo } from "../types.ts";
import { debug } from "../logger.ts";
import {
  BATCH_PROOF_GAS_LIMIT,
  BATCH_VERIFY_SIG,
  MIN_PRIORITY_FEE_GWEI,
  PRECOMPILE_ADDRESS,
  PROOF_API_BASE_DELAY_MS,
  PROOF_API_MAX_RETRIES,
  PROOF_API_TIMEOUT_MS,
  RECEIPT_TIMEOUT_MS,
  RPC_TIMEOUT_MS,
  SINGLE_PROOF_GAS_LIMIT,
  SINGLE_VERIFY_SIG,
} from "../constants.ts";
import { sleep } from "../utils/reconnect.ts";
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

function getErrorMessage(error: unknown): string {
  if (!error) return "";
  if (error instanceof Error) return error.message;
  if (typeof error === "object") {
    const e = error as Record<string, unknown>;
    return `${e.message ?? ""} ${e.shortMessage ?? ""}`;
  }
  return String(error);
}

// ============================================================================
// Utilities
// ============================================================================

function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  label: string,
): Promise<T> {
  const timeout = new Promise<never>((_, reject) =>
    setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms)
  );
  return Promise.race([promise, timeout]);
}

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

  const execute = async (entry: SignerEntry) => {
    const { signer, provider } = entry;

    // Simulate first
    const from = await signer.getAddress();
    try {
      await withTimeout(
        provider.call({ to: PRECOMPILE_ADDRESS, data, from }),
        RPC_TIMEOUT_MS,
        `${label} simulation`,
      );
    } catch (error) {
      const iface = new ethers.Interface(PRECOMPILE_ABI);
      const revertData = (error as { data?: string }).data;
      if (revertData) {
        const parsed = iface.parseError(revertData);
        throw new Error(`${label} will revert: ${parsed?.name ?? "Unknown"}`);
      }
      throw new Error(`${label} simulation failed: ${getErrorMessage(error)}`);
    }

    // Send transaction
    const feeOverrides = await getFeeOverrides(provider);
    debug(`${label} fees`, { ...feeOverrides });

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
      parsed.message ?? text ?? response.statusText,
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

async function fetchProofWithRetry(
  url: string,
  label: string,
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<ProofResponse> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await fetchProof(url);
    } catch (error) {
      const isRetriable =
        (error instanceof Error && error.message.includes("timed out")) ||
        (error instanceof ProofApiError &&
          (error.retriable || error.status >= 500));

      if (attempt < maxRetries - 1 && isRetriable) {
        const delayMs = PROOF_API_BASE_DELAY_MS * Math.pow(2, attempt) +
          Math.random() * 250;
        console.log(
          `⚠️  Proof API retry (${label}, ${attempt + 1}/${maxRetries})...`,
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

  try {
    return await fetchProofWithRetry(indexUrl, "proof-by-index", maxRetries);
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
      return await fetchProofWithRetry(hashUrl, "proof-by-tx", maxRetries);
    }
    throw error;
  }
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
