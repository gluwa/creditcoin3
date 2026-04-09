/**
 * Proof API client — fetch, retry, and convert proof responses.
 */

import { ethers } from "ethers";
import type {
  BatchProofResponse,
  FormattedProof,
  ProofQuery,
  ProofResponse,
  TxInfo,
} from "../types.ts";
import {
  PROOF_API_BASE_DELAY_MS,
  PROOF_API_MAX_RETRIES,
  PROOF_API_TIMEOUT_MS,
} from "../constants.ts";
import { isRetriableFetchError, ProofApiError } from "../utils/errors.ts";
import { sleep } from "../utils/sleep.ts";
import { submitSingleToPrecompile } from "./precompile.ts";

// ============================================================================
// Single Proof Fetch
// ============================================================================

async function fetchProof(url: string): Promise<ProofResponse> {
  console.debug("Proof API request", { url });

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

// ============================================================================
// Retry Logic
// ============================================================================

/** Generic retry wrapper for proof API calls with exponential backoff. */
async function fetchWithRetry<T>(
  fn: () => Promise<T>,
  label: string,
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<T> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await fn();
    } catch (error) {
      if (attempt < maxRetries - 1 && isRetriableFetchError(error)) {
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
  throw new Error(
    `Failed to fetch proof (${label}) after ${maxRetries} attempts`,
  );
}

// ============================================================================
// Public: Fetch Proof for Transaction
// ============================================================================

export async function fetchProofForTx(
  apiUrl: string,
  chainKey: number,
  txInfo: TxInfo,
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<ProofResponse> {
  console.debug("Fetching proof", {
    chainKey,
    block: txInfo.blockNumber,
    txIndex: txInfo.txIndex,
  });

  const indexUrl =
    `${apiUrl}/api/v1/proof/${chainKey}/${txInfo.blockNumber}/${txInfo.txIndex}`;

  try {
    return await fetchWithRetry(
      () => fetchProof(indexUrl),
      `proof-by-index block=${txInfo.blockNumber} txIndex=${txInfo.txIndex}`,
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
      return await fetchWithRetry(
        () => fetchProof(hashUrl),
        `proof-by-tx block=${txInfo.blockNumber} tx=${
          txInfo.txHash.slice(0, 10)
        }`,
        maxRetries,
      );
    }
    throw error;
  }
}

// ============================================================================
// Batch Proof Fetch
// ============================================================================

async function fetchBatchProof(
  url: string,
  queries: ProofQuery[],
): Promise<BatchProofResponse> {
  console.debug("Batch Proof API request", { url, queries: queries.length });

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

export function fetchBatchProofWithRetry(
  apiUrl: string,
  chainKey: number,
  queries: ProofQuery[],
  maxRetries = PROOF_API_MAX_RETRIES,
): Promise<BatchProofResponse> {
  const url = `${apiUrl}/api/v1/proof-batch/${chainKey}`;
  return fetchWithRetry(
    () => fetchBatchProof(url, queries),
    `batch(${queries.length} queries)`,
    maxRetries,
  );
}

// ============================================================================
// Proof Conversion
// ============================================================================

export function convertProofFormat(apiProof: ProofResponse): FormattedProof {
  if (!apiProof.merkleProof) {
    throw new Error("Merkle proof missing from API response");
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
// High-Level: Fetch and Submit
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

  return submitSingleToPrecompile(
    cc3HttpUrl,
    privateKey,
    chainKey,
    apiProof.headerNumber,
    ethers.getBytes(apiProof.txBytes),
    proof,
  );
}
