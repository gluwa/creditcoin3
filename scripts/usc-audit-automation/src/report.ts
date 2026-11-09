/**
 * Report line formatting, kept free of I/O so it can be tested.
 */

export function formatNum(n: number): string {
  return n.toLocaleString("en-US");
}

export interface BlockDiffDetail {
  ok: boolean;
  /** Source chain tip. */
  tip: number;
  /** Last attested source block. */
  attested: number;
  /** Max allowed `tip - attested`. */
  maxBlockDiff: number;
  /** Set for `rpcTag` maturities: the tag and the observed `tip - tagBlock` lag. */
  tag?: { name: "safe" | "finalized"; lag: number };
}

/**
 * The attestation-lag report line. Tag maturities are reported against the tag, where the
 * allowance is stable (`maxBlockDiff - lag`); offset maturities against the tip.
 */
export function formatBlockDiffDetail(d: BlockDiffDetail): string {
  const mark = d.ok ? "✅" : "❌";
  if (!d.tag) {
    return `${mark} Attestation block heights diff: ${
      formatNum(d.tip - d.attested)
    } (${formatNum(d.tip)}|${formatNum(d.attested)}|${
      formatNum(d.maxBlockDiff)
    })`;
  }
  const { name } = d.tag;
  // Clamped as in getMaxBlockDiff: a load-balanced RPC can report a tag above its own tip.
  const lag = Math.max(d.tag.lag, 0);
  const tagBlock = d.tip - lag;
  const behind = tagBlock - d.attested;
  const allowed = d.maxBlockDiff - lag;
  const heights = `latest ${formatNum(d.tip)}|${name} ${
    formatNum(tagBlock)
  }|attested ${formatNum(d.attested)}`;
  return behind >= 0
    ? `${mark} Attestation behind ${name}: ${formatNum(behind)} (allowed ${
      formatNum(allowed)
    }; ${heights})`
    : `${mark} Attestation ahead of ${name} by ${formatNum(-behind)} (allowed ${
      formatNum(allowed)
    } behind; ${heights})`;
}
