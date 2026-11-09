import { assertEquals } from "@std/assert";
import { formatBlockDiffDetail } from "./report.ts";

Deno.test("formatBlockDiffDetail keeps the tip-based line for offset maturities", () => {
  assertEquals(
    formatBlockDiffDetail({
      ok: true,
      tip: 12_076_460,
      attested: 12_076_410,
      maxBlockDiff: 104,
    }),
    "✅ Attestation block heights diff: 50 (12,076,460|12,076,410|104)",
  );
});

Deno.test("formatBlockDiffDetail measures tag maturities from the tag", () => {
  // 6 Nov 2026 asc-devnet Base Sepolia: finalized lag 5,394, allowed 3,720.
  assertEquals(
    formatBlockDiffDetail({
      ok: true,
      tip: 55_224_826,
      attested: 55_217_400,
      maxBlockDiff: 9_114,
      tag: { name: "finalized", lag: 5_394 },
    }),
    "✅ Attestation behind finalized: 2,032 (allowed 3,720; latest 55,224,826|finalized 55,219,432|attested 55,217,400)",
  );
});

Deno.test("formatBlockDiffDetail reports an over-limit attestation", () => {
  assertEquals(
    formatBlockDiffDetail({
      ok: false,
      tip: 12_076_600,
      attested: 12_076_410,
      maxBlockDiff: 109,
      tag: { name: "safe", lag: 47 },
    }),
    "❌ Attestation behind safe: 143 (allowed 62; latest 12,076,600|safe 12,076,553|attested 12,076,410)",
  );
});

Deno.test("formatBlockDiffDetail clamps a negative tag lag like getMaxBlockDiff", () => {
  // The RPC reported its tag 3 blocks above its tip; getMaxBlockDiff treated the lag as 0.
  assertEquals(
    formatBlockDiffDetail({
      ok: true,
      tip: 141_759_364,
      attested: 141_759_300,
      maxBlockDiff: 332,
      tag: { name: "safe", lag: -3 },
    }),
    "✅ Attestation behind safe: 64 (allowed 332; latest 141,759,364|safe 141,759,364|attested 141,759,300)",
  );
});

Deno.test("formatBlockDiffDetail says ahead when the RPC's tag trails the attestation", () => {
  // The audit's RPC can sit behind the attestors' node, so its tag can be below the attestation.
  assertEquals(
    formatBlockDiffDetail({
      ok: true,
      tip: 56_603_600,
      attested: 56_598_000,
      maxBlockDiff: 10_602,
      tag: { name: "finalized", lag: 6_882 },
    }),
    "✅ Attestation ahead of finalized by 1,282 (allowed 3,720 behind; latest 56,603,600|finalized 56,596,718|attested 56,598,000)",
  );
});

Deno.test("formatBlockDiffDetail says why an attestation above the RPC's tip fails", () => {
  // The check requires tip - attested >= 0; an attestation above this RPC's tip fails it.
  assertEquals(
    formatBlockDiffDetail({
      ok: false,
      tip: 56_603_600,
      attested: 56_604_200,
      maxBlockDiff: 10_602,
      tag: { name: "finalized", lag: 6_882 },
    }),
    "❌ Attestation above latest by 600 (latest 56,603,600|finalized 56,596,718|attested 56,604,200)",
  );
});
