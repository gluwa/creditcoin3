/**
 * Maturity strategy resolution.
 *
 * Mirrors `MaturityStrategy` in primitives/supported-chains: the strategy is stored on-chain as a
 * string and is parsed here into either a fixed block offset or a source-chain RPC block tag.
 */

export type RpcBlockTag = "safe" | "finalized";

export type Maturity =
  | { kind: "offset"; delay: number }
  | { kind: "rpcTag"; tag: RpcBlockTag };

/** Runtime default (`DefaultMaturityStrategy` in runtime/src/lib.rs). */
export const DEFAULT_MATURITY_STRATEGY = "EvmSafe";

/** Extra slack for tag strategies: the tag jumps by up to an epoch (32 blocks) between attestations. */
export const RPC_TAG_STEP_BUFFER = 32;

/** Used when a tag strategy's block cannot be fetched; `EvmSafe` is too tight for `tip - safe`. */
export const TAG_FALLBACK_STRATEGY = "EvmFinalized";

/** Attestations may lag the mature height by up to this many attestation intervals. */
export const ATTESTATION_LAG_BUFFER_INTERVALS = 3;

/** Parses an on-chain strategy string; returns null if it is not a valid strategy. */
export function parseMaturity(strategy: string): Maturity | null {
  switch (strategy) {
    case "EvmFinalized":
      return { kind: "offset", delay: 64 };
    case "EvmSafe":
      return { kind: "offset", delay: 32 };
    case "EvmLatest":
      return { kind: "offset", delay: 0 };
    case "RpcSafe":
      return { kind: "rpcTag", tag: "safe" };
    case "RpcFinalized":
      return { kind: "rpcTag", tag: "finalized" };
  }
  const m = strategy.match(/^FixedDelay:\s*(\d+)\s*$/);
  return m ? { kind: "offset", delay: Number(m[1]) } : null;
}

export interface ResolvedMaturity {
  /** Strategy string actually used, suffixed with " (fallback)" when defaulted. */
  label: string;
  maturity: Maturity;
  warning?: string;
}

/** Resolves the on-chain strategy, falling back to the default (with a warning) if absent or invalid. */
export function resolveMaturity(
  strategy: string | null | undefined,
): ResolvedMaturity {
  const parsed = strategy ? parseMaturity(strategy) : null;
  if (strategy && parsed) return { label: strategy, maturity: parsed };

  const fallback = parseMaturity(DEFAULT_MATURITY_STRATEGY)!;
  return {
    label: `${DEFAULT_MATURITY_STRATEGY} (fallback)`,
    maturity: fallback,
    warning: strategy
      ? `unknown maturity strategy "${strategy}", falling back to ${DEFAULT_MATURITY_STRATEGY}`
      : `no on-chain maturity strategy found, falling back to ${DEFAULT_MATURITY_STRATEGY}`,
  };
}

/**
 * Max allowed gap between the source chain tip and the last attested block.
 * `tagLag` is the observed `tip - taggedBlock` gap, required for `rpcTag` maturities.
 */
export function getMaxBlockDiff(
  maturity: Maturity,
  attestationInterval: number,
  tagLag?: number,
): number {
  const maturityLag = maturity.kind === "offset"
    ? maturity.delay
    : Math.max(tagLag ?? 0, 0) + RPC_TAG_STEP_BUFFER;
  return maturityLag + attestationInterval * ATTESTATION_LAG_BUFFER_INTERVALS;
}
