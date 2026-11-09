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
 * on OP-stack L2s such as Base, advance in steps of about this long.
 */
export const EPOCH_SECONDS = 384;

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
 * How far a `safe` / `finalized` tag can jump between two reads, in source-chain blocks: one epoch
 * of the chain's blocks, capped by how far the tag trails the tip, and never below
 * `RPC_TAG_STEP_BUFFER`.
 *
 * - Ethereum (12 s blocks): an epoch is 32 blocks, so the step stays 32.
 * - BSC (sub-second blocks, fast finality): the tag trails the tip by a block or two and moves
 *   almost every block, so the cap keeps the step at 32.
 * - Base (0.2 s blocks): the tag follows L1 finality, trails the tip by thousands of blocks and
 *   jumps about 1,920 blocks per epoch, so the step is 1,920.
 *
 * Without a usable `blockTimeSecs` the step is `RPC_TAG_STEP_BUFFER`.
 */
export function getTagStepBuffer(
  tagLag: number,
  blockTimeSecs?: number,
): number {
  if (blockTimeSecs == null || !(blockTimeSecs > 0)) return RPC_TAG_STEP_BUFFER;
  const epochBlocks = Math.ceil(EPOCH_SECONDS / blockTimeSecs);
  return Math.max(
    RPC_TAG_STEP_BUFFER,
    Math.min(epochBlocks, Math.max(tagLag, 0)),
  );
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
  const lag = Math.max(tagLag ?? 0, 0);
  const maturityLag = maturity.kind === "offset"
    ? maturity.delay
    : lag + getTagStepBuffer(lag, blockTimeSecs);
  return maturityLag + attestationInterval * ATTESTATION_LAG_BUFFER_INTERVALS;
}
