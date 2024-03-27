import execa = require('execa');
import { commandSync } from 'execa';

import type { EventRecord, Balance, DispatchError } from '../lib';
import { ApiPromise, expectNoDispatchError, newApi } from '../lib';

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

export async function expectIsFinalizing() {
    // note: create this object here b/c the calling environment is just outside
    // the test suite and we don't yet have access to an API object :-(
    const api = (await newApi((global as any).CREDITCOIN_API_URL)).api;

    const [lastBlockNumber, finalized] = await Promise.all([
        getCreditcoinBlockNumber(api),
        api.rpc.chain.getBlock(await api.rpc.chain.getFinalizedHead()),
    ]);

    const lastFinalizedNumber = finalized.block.header.number.toNumber();

    // tolerate at most 5 blocks difference
    expect(lastBlockNumber - lastFinalizedNumber).toBeLessThanOrEqual(5);
}

function runNode(extraArgs: string) {
    // warning: do NOT await, runs in background
    void execa(
        '../target/release/creditcoin3-node',
        `--chain dev --validator --pruning archive ${extraArgs}`.split(' '),
        {
            detached: true,
            stdout: 'ignore',
            stderr: 'ignore',
        },
    );
}

export async function startAliceAndBob() {
    console.log('INFO: starting creditcoin3-node processes for Alice and Bob');

    // possible restart between multiple tests
    // waitfor network sockets to recycle and avoid messages like
    // disconnected from ws://127.0.0.1:9944: 1006:: Abnormal Closure
    await sleep(2000);

    runNode('--alice --tmp --node-key d182d503b7dd97e7c055f33438c7717145840fd66b2a055284ee8d768241a463');
    await sleep(2000);

    runNode(
        '--bob --tmp --bootnodes /ip4/127.0.0.1/tcp/30333/p2p/12D3KooWKEKymnBDKfa8MkMWiLE6DYbC4aAUciqmYucm7xFKK3Au --port 30335 --rpc-port 9955',
    );
    await sleep(1000);
}

export function killCreditcoinNodes() {
    console.log('INFO: killing all creditcoin3-node processes');

    commandSync(`killall -9 creditcoin3-node`);
}
