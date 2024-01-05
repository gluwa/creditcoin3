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
