import { spawn, type ChildProcess } from "node:child_process";
import { closeSync, mkdirSync, openSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

export class FatalError extends Error {}

async function bounded<T>(
  promise: Promise<T>,
  timeoutMs: number,
  signal?: AbortSignal,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort: (() => void) | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error("probe exceeded its remaining deadline")),
      timeoutMs,
    );
    abort = () => reject(signal?.reason ?? new Error("aborted"));
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) abort();
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timer);
    if (abort) signal?.removeEventListener("abort", abort);
  }
}

export async function waitFor<T>(
  label: string,
  probe: () => Promise<T | undefined>,
  options: {
    timeoutMs: number;
    intervalMs?: number;
    signal?: AbortSignal;
    check?: () => void;
  },
): Promise<T> {
  const deadline = Date.now() + options.timeoutMs;
  let lastError: unknown;
  while (Date.now() < deadline) {
    options.signal?.throwIfAborted();
    options.check?.();
    try {
      const result = await bounded(
        probe(),
        Math.max(1, deadline - Date.now()),
        options.signal,
      );
      if (result !== undefined) return result;
    } catch (error) {
      if (error instanceof FatalError) throw error;
      lastError = error;
    }
    options.signal?.throwIfAborted();
    await delay(
      Math.min(options.intervalMs ?? 1000, Math.max(0, deadline - Date.now())),
      undefined,
      {
        signal: options.signal,
      },
    );
  }
  throw new Error(
    `Timed out waiting for ${label}${lastError ? `: ${String(lastError)}` : ""}`,
  );
}

export interface Exit {
  code: number | null;
  signal: NodeJS.Signals | null;
  error?: string;
}

/** Each child owns a process group. Cleanup never searches or kills unrelated processes. */
export class ManagedProcess {
  readonly child: ChildProcess;
  readonly finished: Promise<Exit>;
  exit?: Exit;
  expectedExit = false;
  private stopped = false;

  constructor(
    readonly name: string,
    command: string,
    args: string[],
    readonly log: string,
    options: { cwd: string; env?: NodeJS.ProcessEnv },
  ) {
    mkdirSync(dirname(log), { recursive: true });
    const fd = openSync(log, "a");
    try {
      this.child = spawn(command, args, {
        ...options,
        detached: true,
        stdio: ["ignore", fd, fd],
      });
    } finally {
      closeSync(fd);
    }
    this.finished = new Promise((resolve) => {
      this.child.once("error", (error) => {
        this.exit = { code: -1, signal: null, error: error.message };
        resolve(this.exit);
      });
      this.child.once("exit", (code, signal) => {
        this.exit = { code, signal };
        resolve(this.exit);
      });
    });
  }

  assertAlive(): void {
    if (this.exit && !this.expectedExit) {
      throw new FatalError(
        `${this.name} exited unexpectedly: ${JSON.stringify(this.exit)}; see ${this.log}`,
      );
    }
  }

  private signalGroup(signal: NodeJS.Signals): void {
    if (!this.child.pid) return;
    try {
      process.kill(-this.child.pid, signal);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
    }
  }

  async stop(signal: NodeJS.Signals = "SIGINT"): Promise<Exit> {
    if (this.stopped) return this.exit!;
    this.expectedExit = true;
    this.signalGroup(signal);
    let exit = this.exit;
    if (!exit) {
      try {
        exit = await bounded(this.finished, signal === "SIGKILL" ? 3000 : 8000);
      } catch {
        /* Escalate only this process group below. */
      }
    }
    // The leader can exit before its descendants; always clean its owned process group too.
    this.signalGroup("SIGKILL");
    const finished = exit ?? (await bounded(this.finished, 3000));
    this.stopped = true;
    return finished;
  }

  async runToCompletion(
    timeoutMs: number,
    signal?: AbortSignal,
  ): Promise<void> {
    this.expectedExit = true;
    try {
      const exit = await waitFor(
        `${this.name} completion`,
        async () => this.exit,
        { timeoutMs, signal },
      );
      if (exit.code !== 0)
        throw new Error(
          `${this.name} failed: ${JSON.stringify(exit)}; see ${this.log}`,
        );
    } finally {
      await this.stop();
    }
  }
}

export async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await readFile(path, "utf8")) as T;
}

export function json(value: unknown): string {
  return JSON.stringify(
    value,
    (_, item: unknown) => (typeof item === "bigint" ? item.toString() : item),
    2,
  );
}

export interface CaseResult {
  name: string;
  seconds: number;
  status: "passed" | "failed" | "skipped";
  error?: string;
  evidence?: unknown;
}

const xml = (text: string) =>
  text.replace(
    /[<>&"']/g,
    (char) =>
      ({
        "<": "&lt;",
        ">": "&gt;",
        "&": "&amp;",
        '"': "&quot;",
        "'": "&apos;",
      })[char]!,
  );

export function junit(cases: CaseResult[]): string {
  return `<?xml version="1.0" encoding="UTF-8"?>\n<testsuite name="writeability-fullstack" tests="${cases.length}" failures="${cases.filter((c) => c.status === "failed").length}" skipped="${cases.filter((c) => c.status === "skipped").length}">\n${cases.map((c) => `  <testcase name="${xml(c.name)}" time="${c.seconds}">${c.status === "failed" ? `<failure message="${xml(c.error ?? "failed")}">${xml(c.error ?? "failed")}</failure>` : c.status === "skipped" ? "<skipped/>" : ""}</testcase>`).join("\n")}\n</testsuite>\n`;
}

export async function writeReports(
  dir: string,
  cases: CaseResult[],
  details: unknown,
): Promise<void> {
  await writeFile(`${dir}/report.json`, json({ cases, details }));
  await writeFile(`${dir}/junit.xml`, junit(cases));
}
