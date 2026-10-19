import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { FatalError, ManagedProcess, junit, waitFor } from "./runtime.mjs";

test("waits enforce deadline and cancellation during an unresponsive probe", async () => {
  const never = () => new Promise<undefined>(() => {});
  const start = Date.now();
  await assert.rejects(
    waitFor("blackhole", never, { timeoutMs: 30 }),
    /blackhole/,
  );
  assert.ok(Date.now() - start < 1000);
  const controller = new AbortController();
  const waiting = waitFor("cancel", never, {
    timeoutMs: 10_000,
    signal: controller.signal,
  });
  controller.abort(new Error("test cancellation"));
  await assert.rejects(waiting, /test cancellation/);
  await assert.rejects(
    waitFor(
      "fatal",
      async () => {
        throw new FatalError("invalid evidence");
      },
      { timeoutMs: 10_000 },
    ),
    /invalid evidence/,
  );
  assert.equal(
    await waitFor("zero is evidence", async () => 0, { timeoutMs: 100 }),
    0,
  );
});

test("SIGKILL targets the managed process group and leaves another child alive", async () => {
  const dir = await mkdtemp(join(tmpdir(), "e2e-process-test-"));
  const create = (name: string) =>
    new ManagedProcess(
      name,
      process.execPath,
      ["-e", "console.log('ready');setInterval(() => {}, 1000)"],
      join(dir, `${name}.log`),
      { cwd: dir },
    );
  const killed = create("killed");
  const survivor = create("survivor");
  try {
    await waitFor(
      "children ready",
      async () =>
        (await readFile(killed.log, "utf8")).includes("ready") &&
        (await readFile(survivor.log, "utf8")).includes("ready")
          ? true
          : undefined,
      { timeoutMs: 5000, intervalMs: 10 },
    );
    const exit = await killed.stop("SIGKILL");
    assert.equal(exit.signal, "SIGKILL");
    assert.equal(survivor.exit, undefined);
    survivor.assertAlive();
    assert.deepEqual(
      await killed.stop(),
      exit,
      "cleanup must be idempotent and not revisit an old PID",
    );
  } finally {
    await killed.stop();
    await survivor.stop();
    await rm(dir, { recursive: true, force: true });
  }
});

test("JUnit diagnostics escape errors and preserve failed/skipped scenario counts", () => {
  const report = junit([
    {
      name: "crash & restart",
      status: "failed",
      seconds: 1,
      error: '<bad "RPC">',
    },
    { name: "later", status: "skipped", seconds: 0 },
  ]);
  assert.match(report, /failures="1" skipped="1"/);
  assert.match(report, /crash &amp; restart/);
  assert.match(report, /&lt;bad &quot;RPC&quot;&gt;/);
  assert.match(report, /<skipped\/>/);
});

test("runner validates scenario selection and lookback before starting a stack", () => {
  const run = (...args: string[]) =>
    spawnSync(
      process.execPath,
      [
        "--import",
        "tsx",
        fileURLToPath(new URL("run.mts", import.meta.url)),
        ...args,
      ],
      {
        encoding: "utf8",
        timeout: 5000,
      },
    );
  const help = run("--help");
  assert.equal(help.status, 0);
  assert.match(help.stdout, /durable-restart/);
  for (const scenario of [
    "missing",
    "constructor",
    "happy-path,happy-path",
    "",
  ]) {
    const result = run("--scenario", scenario);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /unknown or duplicate scenarios/);
  }
  const badLookback = run("--lookback-blocks", "0");
  assert.equal(badLookback.status, 1);
  assert.match(badLookback.stderr, /lookback-blocks/);
});
