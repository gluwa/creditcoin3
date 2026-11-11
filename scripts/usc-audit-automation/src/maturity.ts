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

/** Minimum slack for tag strategies: one Ethereum epoch (32 blocks), the step of an L1 tag. */
export const RPC_TAG_STEP_BUFFER = 32;

/**
 * One Ethereum epoch (32 slots of 12 s). L1-anchored `safe` / `finalized` tags, on Ethereum and
 * on OP-stack L2s such as Base, advance in steps of about this long. Assumes 12 s L1 slots: a
 * shorter slot would make this, and the epoch step on L2s, larger than needed (looser, not
 * stricter).
 */
export const EPOCH_SECONDS = 384;

/** Shorter measured block times are treated as an RPC fault and ignored (Base is 0.2 s). */
export const MIN_PLAUSIBLE_BLOCK_TIME_SECS = 0.05;

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
 * The observed `tip - taggedBlock` lag as the checks use it. A load-balanced RPC can report a tag
 * above its own tip; that counts as no lag.
 */
export function clampTagLag(tagLag: number | undefined): number {
  return Math.max(tagLag ?? 0, 0);
}

/**
 * How far a `safe` / `finalized` tag can jump between two reads, in source-chain blocks.
 *
 * A tag that trails the tip by at least one epoch of the chain's blocks is following L1
 * finality and jumps by that epoch: Base (0.2 s blocks, finalized thousands of blocks behind)
 * gets 1,920. Any other tag gets `RPC_TAG_STEP_BUFFER`: Ethereum (12 s, an epoch is 32 blocks),
 * BSC (fast finality, a block or two behind) and Base `safe` (a few hundred behind).
 *
 * Without a plausible `blockTimeSecs` the step is `RPC_TAG_STEP_BUFFER`.
 */
export function getTagStepBuffer(
  tagLag: number,
  blockTimeSecs?: number,
): number {
  if (
    blockTimeSecs == null ||
    !(blockTimeSecs >= MIN_PLAUSIBLE_BLOCK_TIME_SECS)
  ) {
    return RPC_TAG_STEP_BUFFER;
  }
  const epochBlocks = Math.ceil(EPOCH_SECONDS / blockTimeSecs);
  return clampTagLag(tagLag) >= epochBlocks
    ? Math.max(epochBlocks, RPC_TAG_STEP_BUFFER)
    : RPC_TAG_STEP_BUFFER;
}

/**
 * Max allowed gap between the source chain tip and the last attested block.
 * `tagLag` is the observed `tip - taggedBlock` gap, required for `rpcTag` maturities.
 * `blockTimeSecs` is the source chain's average block time, which sizes the tag step.
 */
export function getMaxBlockDiff(
  maturity: Maturity,
  attestationInterval: number,
  tagLag?: number,
  blockTimeSecs?: number,
): number {
  const lag = clampTagLag(tagLag);
  const maturityLag = maturity.kind === "offset"
    ? maturity.delay
    : lag + getTagStepBuffer(lag, blockTimeSecs);
  return maturityLag + attestationInterval * ATTESTATION_LAG_BUFFER_INTERVALS;
}
