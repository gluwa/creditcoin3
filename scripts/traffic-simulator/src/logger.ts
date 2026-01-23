let verboseEnabled = false;

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(
      value,
      (_key, val) => typeof val === "bigint" ? val.toString() : val,
    );
  } catch {
    return String(value);
  }
}

export function setVerbose(enabled: boolean): void {
  verboseEnabled = enabled;
}

export function debug(message: string, data?: Record<string, unknown>): void {
  if (!verboseEnabled) {
    return;
  }
  const meta = data ? ` ${safeStringify(data)}` : "";
  console.log(`🔍 ${message}${meta}`);
}
