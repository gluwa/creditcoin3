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

interface FetchProofContext {
  blockNumber: number;
  /** Transaction index (for proof-by-index) or hash (for proof-by-tx) */
  txIdentifier: number | string;
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
      if (attempt < maxRetries - 1 && isRetriableFetchError(error)) {
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

export async function fetchBatchProofWithRetry(
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
      if (attempt < maxRetries - 1 && isRetriableFetchError(error)) {
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
