import { assertEquals } from "@std/assert";
import { getMaxBlockDiff, parseMaturity, resolveMaturity } from "./maturity.ts";

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
