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

/** Retry only explicit throttling, at the failed read rather than restarting the whole scan. */
export async function retryRateLimited<T>(operation: string, request: () => Promise<T>): Promise<T> {
    for (let attempt = 0; ; attempt++) {
        try {
            return await request();
        } catch (error) {
            if (!isRateLimitError(error)) throw error;
            if (attempt === 4) throw new RateLimitExhausted(operation);
            const delay = 2_000 * 2 ** attempt;
            console.warn(`${operation}: rate limited; retrying in ${delay / 1_000}s`);
            await new Promise((resolve) => setTimeout(resolve, delay));
        }
    }
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
