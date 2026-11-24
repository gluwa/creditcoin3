import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { isRateLimitError, isTransientError, retryRequest, runProverCheck } from '../../scripts/prover-check-support';

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
        const result = retryRequest('decoder RPC', request);
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
            await retryRequest('decoder RPC', request);
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
                await retryRequest('verify proof RPC', request);
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

// Shape returned by ethers for the Sepolia source RPC's 503 in Check Prover run 44519981030.
const ethers503 = {
    code: 'SERVER_ERROR',
    shortMessage: 'server response 503 Service Unavailable',
    info: { responseStatus: '503 Service Unavailable', responseBody: '{"error":{"code":503}}' },
};

describe('prover check transient failures', () => {
    let directory: string;
    const originalSummary = process.env.GITHUB_STEP_SUMMARY;

    beforeEach(() => {
        directory = mkdtempSync(join(tmpdir(), 'prover-check-test-'));
        process.env.GITHUB_STEP_SUMMARY = join(directory, 'summary');
        jest.useFakeTimers();
        jest.spyOn(console, 'warn').mockImplementation(() => undefined);
        jest.spyOn(console, 'error').mockImplementation(() => undefined);
    });

    afterEach(() => {
        jest.useRealTimers();
        jest.restoreAllMocks();
        rmSync(directory, { recursive: true, force: true });
        if (originalSummary === undefined) delete process.env.GITHUB_STEP_SUMMARY;
        else process.env.GITHUB_STEP_SUMMARY = originalSummary;
    });

    test.each([
        ethers503,
        { response: { status: 502 } },
        { cause: { statusCode: 504 } },
        { code: 'ECONNRESET' },
        { code: 'ECONNABORTED' },
        { code: 'TIMEOUT' },
    ])('recognizes a briefly unavailable service: %j', (error) => {
        expect(isTransientError(error)).toBe(true);
    });

    test.each([
        new Error('proof verification failed'),
        { code: 'CALL_EXCEPTION', info: { error: { code: -32603, message: 'execution reverted' } } },
        { response: { status: 500 } },
        { response: { status: 422 } },
        { response: { status: 429 } },
        null,
    ])('does not treat errors, reverts, or throttling as transient: %j', (error) => {
        expect(isTransientError(error)).toBe(false);
    });

    test('retries after 5s and then 10s, and reports a recovery on retry #2', async () => {
        const request = jest
            .fn()
            .mockRejectedValueOnce(ethers503)
            .mockRejectedValueOnce(ethers503)
            .mockResolvedValue(7);
        const result = retryRequest('source block RPC', request);
        await jest.advanceTimersByTimeAsync(4_999);
        expect(request).toHaveBeenCalledTimes(1);
        await jest.advanceTimersByTimeAsync(1);
        expect(request).toHaveBeenCalledTimes(2);
        await jest.advanceTimersByTimeAsync(9_999);
        expect(request).toHaveBeenCalledTimes(2);
        await jest.advanceTimersByTimeAsync(1);
        await expect(result).resolves.toBe(7);
        expect(request).toHaveBeenCalledTimes(3);
        expect(console.warn).toHaveBeenCalledWith(
            expect.stringContaining('::warning::source block RPC: succeeded on retry #2'),
        );
        expect(readFileSync(join(directory, 'summary'), 'utf8')).toContain('succeeded on retry #2');
    });

    test('fails with the original error once both retries are used', async () => {
        const request = jest.fn().mockRejectedValue(ethers503);
        const result = expect(retryRequest('source block RPC', request)).rejects.toBe(ethers503);
        await jest.runAllTimersAsync();
        await result;
        expect(request).toHaveBeenCalledTimes(3);
        expect(existsSync(join(directory, 'summary'))).toBe(false);
    });

    test('a request that succeeds first time leaves no warning', async () => {
        await expect(retryRequest('fetch proof', () => Promise.resolve('ok'))).resolves.toBe('ok');
        expect(console.warn).not.toHaveBeenCalled();
    });
});
