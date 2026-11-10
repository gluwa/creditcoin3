import { assertEquals } from "@std/assert";
import {
  getMaxBlockDiff,
  getTagStepBuffer,
  parseMaturity,
  resolveMaturity,
} from "./maturity.ts";

Deno.test("parseMaturity handles every on-chain strategy", () => {
  assertEquals(parseMaturity("EvmFinalized"), { kind: "offset", delay: 64 });
  assertEquals(parseMaturity("EvmSafe"), { kind: "offset", delay: 32 });
  assertEquals(parseMaturity("EvmLatest"), { kind: "offset", delay: 0 });
  assertEquals(parseMaturity("FixedDelay: 10"), { kind: "offset", delay: 10 });
  assertEquals(parseMaturity("FixedDelay: 10 "), { kind: "offset", delay: 10 });
  assertEquals(parseMaturity("FixedDelay:7"), { kind: "offset", delay: 7 });
  assertEquals(parseMaturity("RpcSafe"), { kind: "rpcTag", tag: "safe" });
  assertEquals(parseMaturity("RpcFinalized"), {
    kind: "rpcTag",
    tag: "finalized",
  });
});

Deno.test("parseMaturity rejects malformed strategies", () => {
  for (const s of ["", "evmsafe", "FixedDelay:", "FixedDelay: x", "Bogus"]) {
    assertEquals(parseMaturity(s), null, s);
  }
});

Deno.test("resolveMaturity keeps valid strategies without warning", () => {
  const r = resolveMaturity("FixedDelay: 5");
  assertEquals(r.label, "FixedDelay: 5");
  assertEquals(r.maturity, { kind: "offset", delay: 5 });
  assertEquals(r.warning, undefined);
});

Deno.test("resolveMaturity falls back to EvmSafe with a warning", () => {
  for (const s of [null, undefined, "", "Bogus"]) {
    const r = resolveMaturity(s);
    assertEquals(r.label, "EvmSafe (fallback)");
    assertEquals(r.maturity, { kind: "offset", delay: 32 });
    assertEquals(typeof r.warning, "string");
  }
});

Deno.test("getMaxBlockDiff uses offset delay or observed tag lag", () => {
  assertEquals(getMaxBlockDiff({ kind: "offset", delay: 32 }, 10), 62);
  assertEquals(getMaxBlockDiff({ kind: "offset", delay: 0 }, 10), 30);
  const tag = { kind: "rpcTag", tag: "finalized" } as const;
  assertEquals(getMaxBlockDiff(tag, 10, 100), 162);
  assertEquals(getMaxBlockDiff(tag, 10, -5), 62);
});

Deno.test("getTagStepBuffer stays at 32 without a usable block time", () => {
  for (const bt of [undefined, 0, -1, NaN]) {
    assertEquals(getTagStepBuffer(5000, bt), 32, String(bt));
  }
});

Deno.test("getTagStepBuffer stays at 32 for an implausibly short block time", () => {
  // A broken RPC reporting near-identical timestamps must not loosen the limit.
  for (const bt of [0.001, 0.01, 0.049]) {
    assertEquals(getTagStepBuffer(100_000, bt), 32, String(bt));
  }
  assertEquals(getTagStepBuffer(100_000, 0.05), 7680);
});

// Block times and tag lags measured on each source chain (Nov 2026, 15 min sample); strategies
// and intervals read from each network's SupportedChains / ChainAttestationInterval storage.
// `allowed` is how far the last attestation may trail the tag: maxBlockDiff - tagLag.
const SOURCE_CHAINS = [
  {
    chain: "Ethereum mainnet finalized",
    bt: 12.04,
    lags: [62, 95],
    interval: 10,
    allowed: 62,
  },
  {
    chain: "Ethereum mainnet safe",
    bt: 12.04,
    lags: [30, 63],
    interval: 10,
    allowed: 62,
  },
  {
    chain: "Ethereum Sepolia finalized",
    bt: 12.28,
    lags: [63, 95],
    interval: 10,
    allowed: 62,
  },
  {
    chain: "Ethereum Sepolia safe",
    bt: 12.28,
    lags: [32, 64],
    interval: 10,
    allowed: 62,
  },
  { chain: "BSC mainnet", bt: 0.45, lags: [0, 2], interval: 100, allowed: 332 },
  { chain: "BSC testnet", bt: 0.45, lags: [0, 2], interval: 100, allowed: 332 },
  {
    chain: "Base Sepolia finalized (asc-devnet)",
    bt: 0.2,
    lags: [4741, 8053],
    interval: 600,
    allowed: 3720,
  },
  {
    chain: "Base Sepolia finalized (cc3-devnet)",
    bt: 0.2,
    lags: [4741, 8053],
    interval: 50,
    allowed: 2070,
  },
  {
    chain: "Base mainnet finalized",
    bt: 0.2,
    lags: [4075, 7784],
    interval: 600,
    allowed: 3720,
  },
] as const;

// The tag only selects which block the caller measured `tagLag` against; the allowance is the same.
const TAG = { kind: "rpcTag", tag: "finalized" } as const;

Deno.test("getMaxBlockDiff allowance per source chain", () => {
  for (const c of SOURCE_CHAINS) {
    for (const lag of c.lags) {
      assertEquals(
        getMaxBlockDiff(TAG, c.interval, lag, c.bt) - lag,
        c.allowed,
        `${c.chain} at tag lag ${lag}`,
      );
    }
  }
});

const BLOCK_TIMES = [
  undefined,
  0.001,
  0.05,
  0.2,
  0.25,
  0.45,
  0.75,
  2,
  12,
  12.3,
  15,
];
const INTERVALS = [1, 10, 50, 100, 600];

Deno.test("getMaxBlockDiff is never stricter than the fixed 32-block step", () => {
  for (const bt of BLOCK_TIMES) {
    for (const interval of INTERVALS) {
      for (let lag = -50; lag <= 20_000; lag += 7) {
        const fixedStep = Math.max(lag, 0) + 32 + 3 * interval;
        const now = getMaxBlockDiff(TAG, interval, lag, bt);
        assertEquals(
          now >= fixedStep,
          true,
          `bt ${bt} I ${interval} lag ${lag}`,
        );
      }
    }
  }
});

Deno.test("getTagStepBuffer keeps 32 until the tag trails by a full epoch", () => {
  // BSC (0.45 s, epoch 854 blocks): a finality stall with the tag 500 behind keeps 32.
  assertEquals(getTagStepBuffer(500, 0.45), 32);
  assertEquals(getTagStepBuffer(853, 0.45), 32);
  assertEquals(getTagStepBuffer(854, 0.45), 854);
  // Base safe trails by a few hundred blocks, under an epoch: 32 as before.
  assertEquals(getTagStepBuffer(282, 0.2), 32);
  assertEquals(getTagStepBuffer(1357, 0.2), 32);
  // Base finalized trails by thousands: one epoch.
  assertEquals(getTagStepBuffer(1919, 0.2), 32);
  assertEquals(getTagStepBuffer(1920, 0.2), 1920);
  assertEquals(getTagStepBuffer(5394, 0.2), 1920);
});

Deno.test("getTagStepBuffer is 32 for Ethereum-speed chains at any lag", () => {
  for (const bt of [12, 12.024, 12.3, 15]) {
    for (let lag = 0; lag <= 10_000; lag += 3) {
      assertEquals(getTagStepBuffer(lag, bt), 32, `bt ${bt} lag ${lag}`);
    }
  }
});

Deno.test("getMaxBlockDiff passes the 6 Nov 2026 asc-devnet Base Sepolia report", () => {
  // tip 55,224,826, last attested 55,217,400, finalized lag 5,394, interval 600.
  const tag = { kind: "rpcTag", tag: "finalized" } as const;
  const diff = 55_224_826 - 55_217_400;
  assertEquals(getMaxBlockDiff(tag, 600, 5394), 7226);
  assertEquals(diff > 7226, true);
  assertEquals(getMaxBlockDiff(tag, 600, 5394, 0.2), 9114);
  assertEquals(diff <= 9114, true);
});
