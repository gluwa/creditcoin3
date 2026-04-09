/**
 * Shared error utilities — centralised error classification and decoding.
 */

import { ethers } from "ethers";
import PRECOMPILE_ABI from "../abi/block_prover.json" with { type: "json" };

// ============================================================================
// Error Message Extraction
// ============================================================================

export function getErrorMessage(error: unknown): string {
  if (!error) return "";
  if (error instanceof Error) return error.message;
  if (typeof error === "object") {
    const e = error as Record<string, unknown>;
    return `${e.message ?? ""} ${e.shortMessage ?? ""}`;
  }
  return String(error);
}

// ============================================================================
// Error Classification
// ============================================================================

export function isContinuityMismatchError(error: unknown): boolean {
  return getErrorMessage(error).includes("Continuity proof does not match");
}

export function isNonceError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  return msg.includes("nonce too low") ||
    msg.includes("nonce has already been used");
}

export function isReplacementUnderpricedError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  return msg.includes("replacement transaction underpriced");
}

export function isTransientNetworkError(error: unknown): boolean {
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

export function isRetriableFetchError(error: unknown): boolean {
  const msg = getErrorMessage(error).toLowerCase();
  // Timeout
  if (msg.includes("timed out")) return true;
  // Network errors
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
  // API 500 or explicitly retriable
  if (
    error instanceof ProofApiError && (error.retriable || error.status >= 500)
  ) {
    return true;
  }
  return false;
}

// ============================================================================
// Revert Decoding
// ============================================================================

/** Error(string) selector: 0x08c379a0. Precompile uses this for revert messages. */
const ERROR_STRING_SELECTOR = "0x08c379a0";

export function decodeRevertMessage(revertData: string): string {
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
// ProofApiError
// ============================================================================

export class ProofApiError extends Error {
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
