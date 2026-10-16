// eslint-disable-next-line @typescript-eslint/no-require-imports
import execa = require('execa');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import fs = require('fs');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import os = require('os');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import path = require('path');

import { commandSync } from 'execa';

import type { EventRecord, Balance, DispatchError } from '../lib';
import { ApiPromise, expectNoEventError, expectNoDispatchError, newApi } from '../lib';
import { getChainStatus } from '../lib/chain/status';

export const describeIf = (condition: boolean, name: string, fn: any) =>
    condition ? describe(name, fn) : describe.skip(name, fn);

export const testIf = (condition: boolean, name: string, fn: any, timeout = 30000) =>
    condition ? test(name, fn, timeout) : test.skip(name, fn, timeout);

export const try_catch_else_finally = function (
    protected_code: any,
    handler_code: any,
    else_code: any,
    finally_code: any = undefined,
) {
    try {
        let success = true;
        try {
            protected_code();
        } catch (error: any) {
            success = false;
            handler_code(error);
        }
        if (success) {
            else_code();
        }
    } finally {
        if (finally_code) {
            finally_code();
        }
    }
};

export const extractFee = async (
    resolve: any,
    reject: any,
    unsubscribe: any,
    api: ApiPromise,
    dispatchError: DispatchError | undefined,
    events: EventRecord[],
    status: any,
): Promise<void> => {
    expectNoDispatchError(api, dispatchError);
    if (events) events.forEach((event) => expectNoEventError(api, event));

    if (status.isInBlock) {
        const balancesWithdraw = events.find(({ event: { method, section } }) => {
            return section === 'balances' && method === 'Withdraw';
        });

        expect(balancesWithdraw).toBeTruthy();

        if (balancesWithdraw) {
            const fee = (balancesWithdraw.event.data[1] as Balance).toBigInt();

            const unsub = await unsubscribe;

            if (unsub) {
                unsub();
                resolve(fee);
            } else {
                reject(new Error('Subscription failed'));
            }
        } else {
            reject(new Error("Fee wasn't found"));
        }
    }
};

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const manualSealing = new WeakMap<ApiPromise, Promise<boolean>>();

// true when the node runs with `--sealing`: it then exposes `engine_createBlock` and only
// produces a block when asked to, so tests can seal blocks instead of waiting for them
export const isManualSealing = (api: ApiPromise): Promise<boolean> => {
    let sealing = manualSealing.get(api);
    if (sealing === undefined) {
        sealing = api.rpc.rpc
            .methods()
            .then(({ methods }) => methods.some((method) => method.toString() === 'engine_createBlock'));
        manualSealing.set(api, sealing);
    }
    return sealing;
};

// seal and finalize one block on a `--sealing` node, including whatever is in the pool
export const sealBlock = async (api: ApiPromise) => {
    await api.rpc.engine.createBlock(true, true);
};

// wait until a certain amount of blocks have elapsed
export const forElapsedBlocks = async (api: ApiPromise, opts?: { minBlocks?: number; maxRetries?: number }) => {
    const { maxRetries = 10, minBlocks = 2 } = opts ?? {};
    const initialCreditcoinBlockNumber = (await getChainStatus(api)).bestNumber;
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();

    let retriesCount = 0;
    let creditcoinBlockNumber = initialCreditcoinBlockNumber;

    // wait a min amount of blocks since the initial call to give time to any pending
    // transactions, e.g. test setup to make it into a block
    while (retriesCount < maxRetries && creditcoinBlockNumber <= initialCreditcoinBlockNumber + minBlocks) {
        await sleep(blockTime);
        creditcoinBlockNumber = (await getChainStatus(api)).bestNumber;
        retriesCount++;
    }
};

function runNode(name: string, extraArgs: string) {
    // warning: GitHub doesn't allow uploading files with colon in their name
    const timeStamp = new Date().toISOString().replaceAll(':', '-');
    const logPrefix = path.join(os.tmpdir(), `creditcoin3-node-${name}-${timeStamp}-log`);

    // warning: do NOT await, runs in background
    // CREDITCOIN_SEALING=manual: blocks come from the ticker below instead of BABE slots
    const sealing = process.env.CREDITCOIN_SEALING ? ` --sealing ${process.env.CREDITCOIN_SEALING}` : '';
    void execa(
        '../target/release/creditcoin3-node',
        `--chain dev --validator --pruning archive${sealing} ${extraArgs}`.split(' '),
        {
            detached: true,
            stdout: fs.openSync(`${logPrefix}.stdout`, 'w'),
            stderr: fs.openSync(`${logPrefix}.stderr`, 'w'),
        },
    );
}

let sealTicker: execa.ExecaChildProcess | undefined;

// Under `--sealing manual` nothing produces blocks on its own and GRANDPA does not run, so
// seal and finalize a block on Alice every CREDITCOIN_SEAL_INTERVAL_MS (default 1 s, a fifth
// of a slot) and finalize the same block on Bob, which only imports it. Transactions then
// finalize within about a second instead of the 10-15 s GRANDPA takes on 5 s slots. Chain
// time still moves one slot per block, so it runs ahead of the wall clock.
//
// It runs as its own process: the CLI tests run the CLI with `commandSync`, which blocks
// this one until the CLI's transaction is finalized, i.e. until the next block is sealed.
function startSealTicker() {
    const interval = Number(process.env.CREDITCOIN_SEAL_INTERVAL_MS ?? 1000) / 1000;
    sealTicker = execa(
        '../.github/seal-blocks.sh',
        ['http://127.0.0.1:9944', String(interval), 'http://127.0.0.1:9955'],
        {
            detached: true,
            stdout: fs.openSync(path.join(os.tmpdir(), 'seal-blocks.log'), 'a'),
            stderr: 'inherit',
        },
    );
    // killed on purpose in killCreditcoinNodes()
    sealTicker.catch(() => undefined);
}

export async function startAliceAndBob() {
    console.log('INFO: starting creditcoin3-node processes for Alice and Bob');

    // possible restart between multiple tests
    // waitfor network sockets to recycle and avoid messages like
    // disconnected from ws://127.0.0.1:9944: 1006:: Abnormal Closure
    await sleep(2000);

    runNode('Alice', '--alice --tmp --node-key d182d503b7dd97e7c055f33438c7717145840fd66b2a055284ee8d768241a463');
    await sleep(2000);

    runNode(
        'Bob',
        '--bob --tmp --node-key d9cccabf274641a185bc94f9f8c3d791f50514277cb3ba8c2aa4d7d6757c661f --bootnodes /ip4/127.0.0.1/tcp/30333/p2p/12D3KooWKEKymnBDKfa8MkMWiLE6DYbC4aAUciqmYucm7xFKK3Au --port 30335 --rpc-port 9955',
    );
    await sleep(1000);

    if (process.env.CREDITCOIN_SEALING) {
        startSealTicker();
    }
}

export function killCreditcoinNodes() {
    console.log('INFO: killing all creditcoin3-node processes');

    if (sealTicker) {
        sealTicker.kill('SIGKILL');
        sealTicker = undefined;
    }

    commandSync(`killall -9 creditcoin3-node`);
}

export async function expectIsFinalizing() {
    // note: create this object here b/c the calling environment is just outside
    // the test suite and we don't yet have access to an API object :-(
    const api = (await newApi((global as any).CREDITCOIN_API_URL)).api;

    const chainStatus = await getChainStatus(api);

    // tolerate at most 5 blocks difference
    expect(chainStatus.bestNumber - chainStatus.bestFinalizedNumber).toBeLessThanOrEqual(5);

    // disconnect b/c Alice will start reporting: Too many connections. Please try again later.
    // which causes the calling beforeEach() to timeout after the 8th .test.ts file is executed
    await api.disconnect();
}

export function randomIntBetween(min: number, max: number) {
    return min + Math.floor(Math.random() * (max - min));
}
