import { appendFileSync, mkdirSync, writeFileSync } from 'fs';
import { join } from 'path';

/** Ethers wraps JSON-RPC throttling in CALL_EXCEPTION, sometimes with "missing revert data". */
export function isRateLimitError(error: unknown, seen = new Set<object>()): boolean {
    if (error === null || typeof error !== 'object' || seen.has(error)) return false;
    seen.add(error);
    const value = error as Record<string, unknown>;
    if (value.code === -32029 || value.status === 429 || value.statusCode === 429) return true;
    if (
        typeof value.message === 'string' &&
        /\brate[ -]limit(?:ed| exceeded| reached|ing)?\b|\btoo many requests\b/i.test(value.message)
    ) {
        return true;
    }
    return ['error', 'info', 'cause', 'response', 'data', 'errors'].some((key) => {
        const nested = value[key];
        return Array.isArray(nested)
            ? nested.some((item) => isRateLimitError(item, seen))
            : isRateLimitError(nested, seen);
    });
}

export class RateLimitExhausted extends Error {
    constructor(operation: string) {
        super(`${operation}: rate limited after 5 attempts; check incomplete`);
        this.name = 'RateLimitExhausted';
    }
}

const TRANSIENT_STATUSES = new Set([502, 503, 504]);
const TRANSIENT_CODES = new Set([
    'TIMEOUT', // ethers
    'ECONNABORTED', // axios timeout
    'ECONNRESET',
    'ECONNREFUSED',
    'ETIMEDOUT',
    'EAI_AGAIN',
    'EPIPE',
]);

/**
 * A service that is briefly unavailable: HTTP 502/503/504, a timeout, or a dropped connection.
 * Ethers reports the HTTP status as a string in `info.responseStatus` ("503 Service Unavailable").
 */
export function isTransientError(error: unknown, seen = new Set<object>()): boolean {
    if (error === null || typeof error !== 'object' || seen.has(error)) return false;
    seen.add(error);
    const value = error as Record<string, unknown>;
    if (typeof value.code === 'string' && TRANSIENT_CODES.has(value.code)) return true;
    for (const status of [value.status, value.statusCode, value.responseStatus]) {
        if (TRANSIENT_STATUSES.has(typeof status === 'string' ? parseInt(status, 10) : (status as number))) {
            return true;
        }
    }
    return ['error', 'info', 'cause', 'response', 'data', 'errors'].some((key) => {
        const nested = value[key];
        return Array.isArray(nested)
            ? nested.some((item) => isTransientError(item, seen))
            : isTransientError(nested, seen);
    });
}

/**
 * Delays before each retry of a transient failure. Deliberately short: a blip recovers within
 * seconds, and anything longer is an outage the check should report rather than wait out.
 */
export const TRANSIENT_RETRY_DELAYS_MS = [5_000, 10_000];

function describeError(error: unknown): string {
    const value = (error ?? {}) as Record<string, unknown>;
    return String(value.shortMessage ?? value.message ?? value.code ?? error);
}

/**
 * Retries the failed request rather than restarting the whole scan:
 * - explicit throttling up to 5 attempts with exponential backoff, then RateLimitExhausted;
 * - transient failures twice, after 5s and 10s, then the original error.
 * A request that only succeeds after a transient retry still passes, but leaves a warning on the
 * run so the blip stays visible. Any other error fails immediately.
 */
export async function retryRequest<T>(operation: string, request: () => Promise<T>): Promise<T> {
    let rateLimited = 0;
    let transient = 0;
    for (;;) {
        try {
            const result = await request();
            if (transient > 0) reportRecovered(operation, transient);
            return result;
        } catch (error) {
            let delay: number;
            if (isRateLimitError(error)) {
                if (++rateLimited === 5) throw new RateLimitExhausted(operation);
                delay = 2_000 * 2 ** (rateLimited - 1);
                console.warn(`${operation}: rate limited; retrying in ${delay / 1_000}s`);
            } else if (isTransientError(error) && transient < TRANSIENT_RETRY_DELAYS_MS.length) {
                delay = TRANSIENT_RETRY_DELAYS_MS[transient++];
                console.warn(
                    `${operation}: ${describeError(error)}; retry #${transient} of ${TRANSIENT_RETRY_DELAYS_MS.length} in ${delay / 1_000}s`,
                );
            } else {
                if (transient > 0) console.error(`${operation}: still failing after ${transient} retries`);
                throw error;
            }
            await new Promise((resolve) => setTimeout(resolve, delay));
        }
    }
}

function reportRecovered(operation: string, retries: number) {
    const message = `${operation}: succeeded on retry #${retries} after a transient failure`;
    console.warn(`::warning::${message}`);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${message}\n`);
}

/** Keep partial proofs for diagnosis, but mark them so downstream jobs cannot treat them as checked. */
export async function runProverCheck(check: () => Promise<void>, proofDir: string): Promise<void> {
    let status = 'complete';
    try {
        await check();
    } catch (error) {
        if (!(error instanceof RateLimitExhausted)) throw error;
        status = 'rate-limited';
        mkdirSync(proofDir, { recursive: true });
        writeFileSync(join(proofDir, 'RATE_LIMITED'), `${error.message}\n`);
        console.warn(`::warning::${error.message}`);
        if (process.env.GITHUB_STEP_SUMMARY) {
            appendFileSync(
                process.env.GITHUB_STEP_SUMMARY,
                `Prover check incomplete: ${error.message}. Dependent checks will skip these proofs.\n`,
            );
        }
    }
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `status=${status}\n`);
}
