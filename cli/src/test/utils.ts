import execa = require('execa');
import fs = require('fs');
import os = require('os');
import path = require('path');

import { commandSync } from 'execa';

import type { EventRecord, Balance, DispatchError } from '../lib';
import { ApiPromise, expectNoDispatchError } from '../lib';

export const describeIf = (condition: boolean, name: string, fn: any) =>
    condition ? describe(name, fn) : describe.skip(name, fn);

export const testIf = (condition: boolean, name: string, fn: any, timeout = 30000) =>
    condition ? test(name, fn, timeout) : test.skip(name, fn, timeout);

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

export const getCreditcoinBlockNumber = async (api: ApiPromise): Promise<number> => {
    const response = await api.rpc.chain.getBlock();
    return response.block.header.number.toNumber();
};

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// wait until a certain amount of blocks have elapsed
export const forElapsedBlocks = async (api: ApiPromise, opts?: { minBlocks?: number; maxRetries?: number }) => {
    const { maxRetries = 10, minBlocks = 2 } = opts ?? {};
    const initialCreditcoinBlockNumber = await getCreditcoinBlockNumber(api);

    let retriesCount = 0;
    let creditcoinBlockNumber = await getCreditcoinBlockNumber(api);

    // wait a min amount of blocks since the initial call to give time to any pending
    // transactions, e.g. test setup to make it into a block
    while (retriesCount < maxRetries && creditcoinBlockNumber <= initialCreditcoinBlockNumber + minBlocks) {
        await sleep(5000);
        creditcoinBlockNumber = await getCreditcoinBlockNumber(api);
        retriesCount++;
    }
};

function runNode(name: string, extraArgs: string) {
    // warning: GitHub doesn't allow uploading files with colon in their name
    const timeStamp = new Date().toISOString().replaceAll(':', '-');
    const logPrefix = path.join(os.tmpdir(), `creditcoin3-node-${name}-${timeStamp}-log`);

    // warning: do NOT await, runs in background
    void execa(
        '../target/release/creditcoin3-node',
        `--chain dev --validator --pruning archive ${extraArgs}`.split(' '),
        {
            detached: true,
            stdout: fs.openSync(`${logPrefix}.stdout`, 'w'),
            stderr: fs.openSync(`${logPrefix}.stderr`, 'w'),
        },
    );
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
        '--bob --tmp --bootnodes /ip4/127.0.0.1/tcp/30333/p2p/12D3KooWKEKymnBDKfa8MkMWiLE6DYbC4aAUciqmYucm7xFKK3Au --port 30335 --rpc-port 9955',
    );
    await sleep(1000);
}

export function killCreditcoinNodes() {
    console.log('INFO: killing all creditcoin3-node processes');

    commandSync(`killall -9 creditcoin3-node`);
}
