import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { Stack } from "./stack.mjs";
import { durableRestart, happyPath } from "./scenarios.mjs";
import { type CaseResult, writeReports } from "./runtime.mjs";

const scenarios = {
  "happy-path": happyPath,
  "durable-restart": durableRestart,
};
const { values } = parseArgs({
  options: {
    scenario: { type: "string", default: "happy-path,durable-restart" },
    "output-dir": {
      type: "string",
      default: resolve(tmpdir(), `writeability-e2e-${Date.now()}`),
    },
    "lookback-blocks": { type: "string", default: "2" },
    help: { type: "boolean", short: "h", default: false },
  },
});
if (values.help) {
  console.log(
    "Usage: npm run e2e -- [--scenario happy-path,durable-restart] [--output-dir DIR] [--lookback-blocks 2]\n" +
      "Requires compiled Creditcoin/attestor/proof-gen/relayer binaries, Foundry and ASC_CONTRACTS_DIR artifacts.\n" +
      "Uses a fresh state/ directory under DIR. Logs, exact-message evidence, report.json and junit.xml are retained.",
  );
} else {
  const selected = values.scenario!.split(",");
  if (
    selected.length === 0 ||
    new Set(selected).size !== selected.length ||
    selected.some((name) => !Object.hasOwn(scenarios, name))
  ) {
    throw new Error(`unknown or duplicate scenarios: ${selected.join(",")}`);
  }
  const lookback = Number(values["lookback-blocks"]);
  if (!Number.isSafeInteger(lookback) || lookback < 1 || lookback > 100)
    throw new Error("lookback-blocks must be an integer in 1..100");
  const output = resolve(values["output-dir"]!);
  await mkdir(output, { recursive: true });
  const controller = new AbortController();
  const interrupt = () =>
    controller.abort(
      new Error("E2E interrupted; stopping owned process groups"),
    );
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", interrupt);
  const stack = new Stack(output, lookback, controller.signal);
  const results: CaseResult[] = [];
  let current = "stack-startup";
  let started = Date.now();
  try {
    await stack.setup();
    results.push({
      name: current,
      status: "passed",
      seconds: (Date.now() - started) / 1000,
    });
    for (const name of selected) {
      current = name;
      started = Date.now();
      const evidence = await scenarios[name as keyof typeof scenarios](stack);
      results.push({
        name,
        status: "passed",
        seconds: (Date.now() - started) / 1000,
        evidence,
      });
      await stack.event("scenario-passed", { name });
    }
  } catch (error) {
    process.exitCode = 1;
    const failure =
      error instanceof Error ? (error.stack ?? error.message) : String(error);
    console.error(failure);
    results.push({
      name: current,
      status: "failed",
      seconds: (Date.now() - started) / 1000,
      error: failure,
    });
    for (const name of selected)
      if (!results.some((result) => result.name === name))
        results.push({ name, status: "skipped", seconds: 0 });
  } finally {
    try {
      await stack.close();
    } catch (error) {
      process.exitCode = 1;
      results.push({
        name: "stack-cleanup",
        status: "failed",
        seconds: 0,
        error: String(error),
      });
    }
    await writeReports(output, results, stack.diagnostics());
    process.removeListener("SIGINT", interrupt);
    process.removeListener("SIGTERM", interrupt);
    console.log(`[e2e] artifacts: ${output}`);
  }
}
