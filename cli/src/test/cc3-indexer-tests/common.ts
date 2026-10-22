import { createClient } from 'graphqurl';
import { ApiPromise } from '../../lib';
import { sleep } from '../utils';

const rawGraphQLQuery = (queryString: string) => {
    return createClient({
        endpoint: (global as any).GRAPHQL_URL,
        headers: {
            /* eslint-disable @typescript-eslint/naming-convention */
            'content-type': 'application/json',
        },
    }).query({
        query: queryString,
    });
};

// The chain's best block number, over the node's HTTP RPC on the same port as its websocket.
const bestBlockNumber = async (): Promise<number> => {
    const url: string = (global as any).CREDITCOIN_API_URL.replace(/^ws/, 'http');
    const response = await fetch(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id: 1, jsonrpc: '2.0', method: 'chain_getHeader', params: [] }),
    });
    const { result } = await response.json();
    return parseInt(result.number, 16);
};

// Block until the indexer has processed the chain's best block as of this call. Tests wait
// a number of blocks after their transactions, and under `--sealing` a block comes every
// second, faster than the indexer catches up, so a block count no longer means the indexer
// has seen the event.
export const waitForIndexer = async (timeoutMs = 25_000): Promise<void> => {
    const target = await bestBlockNumber();
    const deadline = Date.now() + timeoutMs;
    let processed = -1;
    while (Date.now() < deadline) {
        const response = await rawGraphQLQuery('query { _metadata { lastProcessedHeight } }');
        // eslint-disable-next-line no-underscore-dangle
        processed = Number(response?.data?._metadata?.lastProcessedHeight ?? -1);
        if (processed >= target) {
            return;
        }
        await sleep(250);
    }
    throw new Error(`indexer at block ${processed}, chain at ${target} after ${timeoutMs} ms`);
};

// Synchronize new-state queries with the chain tip captured at the start of each call.
export const graphQLQuery = async (queryString: string) => {
    await waitForIndexer();
    return rawGraphQLQuery(queryString);
};

// Read back a record already returned by a synchronized graphQLQuery, without waiting
// for a newer chain tip for every ID. Only use this for the same observed record when
// no intervening mutation or later state is expected. This is not a snapshot read:
// mutable records can still change between requests. New-state and list queries must
// continue to use graphQLQuery.
export const indexedRecordQuery = (queryString: string) => rawGraphQLQuery(queryString);

// The chain's clock at its best block. Under `--sealing`, `waitEras` seals blocks ahead of
// time, so chain time runs ahead of the wall clock: "not in the future" means not later
// than the best block, not not later than `Date.now()`.
export const chainNow = async (api: ApiPromise): Promise<number> => (await api.query.timestamp.now()).toNumber();
