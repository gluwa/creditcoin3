/**
 * Promise timeout helper to prevent indefinite hangs.
 */

/** Resolve after `ms` milliseconds (for retries, backoff, etc.). */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  message?: string,
): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(
      () => reject(new Error(message ?? `Operation timeout after ${ms}ms`)),
      ms,
    );
  });
  return Promise.race([promise, timeout]).finally(() =>
    clearTimeout(timeoutId!)
  );
}
