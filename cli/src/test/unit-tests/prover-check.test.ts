import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { isRateLimitError, retryRateLimited, runProverCheck } from '../../scripts/prover-check-support';

// Shape returned by ethers for the decoder's eth_estimateGas in Check Prover run #359.
const wrappedThrottle = {
    code: 'CALL_EXCEPTION',
    shortMessage: 'missing revert data',
    info: { error: { code: -32029, message: 'Rate limit exceeded' } },
};

describe('prover check throttling', () => {
    let directory: string;
    const originalOutput = process.env.GITHUB_OUTPUT;
    const originalSummary = process.env.GITHUB_STEP_SUMMARY;

    beforeEach(() => {
        directory = mkdtempSync(join(tmpdir(), 'prover-check-test-'));
        process.env.GITHUB_OUTPUT = join(directory, 'output');
        process.env.GITHUB_STEP_SUMMARY = join(directory, 'summary');
        jest.useFakeTimers();
        jest.spyOn(console, 'warn').mockImplementation(() => undefined);
    });

    afterEach(() => {
        jest.useRealTimers();
        jest.restoreAllMocks();
        rmSync(directory, { recursive: true, force: true });
        if (originalOutput === undefined) delete process.env.GITHUB_OUTPUT;
        else process.env.GITHUB_OUTPUT = originalOutput;
        if (originalSummary === undefined) delete process.env.GITHUB_STEP_SUMMARY;
        else process.env.GITHUB_STEP_SUMMARY = originalSummary;
    });

    test.each([
        wrappedThrottle,
        { response: { status: 429 } },
        { cause: { statusCode: 429 } },
        { errors: [{ message: 'Too many requests' }] },
    ])('recognizes explicit throttling through provider wrappers: %j', (error) => {
        expect(isRateLimitError(error)).toBe(true);
    });

    test.each([
        new Error('proof verification failed'),
        new Error('totalGas exceeds the block gas limit'),
        { code: 'CALL_EXCEPTION', info: { error: { code: -32603, message: 'execution reverted' } } },
        { response: { status: 500 } },
        { code: 'ECONNRESET' },
        null,
    ])('does not hide proof, gas, or unrelated service failures: %j', (error) => {
        expect(isRateLimitError(error)).toBe(false);
    });

    test('handles cyclic error wrappers', () => {
        const error: { cause?: unknown } = {};
        error.cause = error;
        expect(isRateLimitError(error)).toBe(false);
    });

    test('retries the failed request after backoff and returns its actual result', async () => {
        const request = jest.fn().mockRejectedValueOnce(wrappedThrottle).mockResolvedValueOnce(123n);
        const result = retryRateLimited('decoder RPC', request);
        await jest.advanceTimersByTimeAsync(1_999);
        expect(request).toHaveBeenCalledTimes(1);
        await jest.advanceTimersByTimeAsync(1);
        await expect(result).resolves.toBe(123n);
        expect(request).toHaveBeenCalledTimes(2);
    });

    test('exhausted throttling is non-failing but marks partial artifacts and reports incomplete', async () => {
        writeFileSync(join(directory, 'partial-proof.txt'), 'diagnostic proof');
        const request = jest.fn().mockRejectedValue(wrappedThrottle);
        const result = runProverCheck(async () => {
            await retryRateLimited('decoder RPC', request);
        }, directory);
        await jest.runAllTimersAsync();
        await expect(result).resolves.toBeUndefined();
        expect(request).toHaveBeenCalledTimes(5);
        expect(readFileSync(join(directory, 'output'), 'utf8')).toBe('status=rate-limited\n');
        expect(readFileSync(join(directory, 'RATE_LIMITED'), 'utf8')).toContain('decoder RPC');
        expect(readFileSync(join(directory, 'summary'), 'utf8')).toContain('incomplete');
        expect(readFileSync(join(directory, 'partial-proof.txt'), 'utf8')).toBe('diagnostic proof');
    });

    test('a genuine error after throttling still fails without a non-failure marker', async () => {
        const failure = new Error('proof verification failed');
        const request = jest.fn().mockRejectedValueOnce(wrappedThrottle).mockRejectedValueOnce(failure);
        const result = expect(
            runProverCheck(async () => {
                await retryRateLimited('verify proof RPC', request);
            }, directory),
        ).rejects.toBe(failure);
        await jest.runAllTimersAsync();
        await result;
        expect(request).toHaveBeenCalledTimes(2);
        expect(existsSync(join(directory, 'RATE_LIMITED'))).toBe(false);
        expect(existsSync(join(directory, 'output'))).toBe(false);
    });

    test('a completed scan reports success without marking its proofs incomplete', async () => {
        await runProverCheck(() => Promise.resolve(), directory);
        expect(readFileSync(join(directory, 'output'), 'utf8')).toBe('status=complete\n');
        expect(existsSync(join(directory, 'RATE_LIMITED'))).toBe(false);
    });
});
