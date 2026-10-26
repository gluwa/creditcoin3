import { ApiPromise } from '../../lib';
import { sleep } from '../utils';

const postJSON = async (url: string, body: unknown, timeoutMs: number) => {
    const response = await fetch(url, {
        method: 'POST',
        // eslint-disable-next-line @typescript-eslint/naming-convention
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) {
        throw new Error(`Request to ${url} failed: HTTP ${response.status}`);
    }
    return response.json();
};

// Follow-up lookups of entities returned by a synchronized query need no new head wait.
// Keep graphQLQuery for reads after chain operations, including successive operations
// within one test: caching the head for a whole test would miss those changes.
export const queryIndexedData = async (queryString: string, timeoutMs = 5_000) => {
    const response = await postJSON((global as any).GRAPHQL_URL, { query: queryString }, timeoutMs);
    if (response.errors?.length) {
        throw new Error(`GraphQL query failed: ${JSON.stringify(response.errors)}`);
    }
    if (!response.data) {
        throw new Error('GraphQL query returned no data');
    }
    return response;
};

// The chain's best block number, over the node's HTTP RPC on the same port as its websocket.
const bestBlockNumber = async (timeoutMs: number): Promise<number> => {
    const url: string = (global as any).CREDITCOIN_API_URL.replace(/^ws/, 'http');
    const response = await postJSON(url, { id: 1, jsonrpc: '2.0', method: 'chain_getHeader', params: [] }, timeoutMs);
    if (response.error) {
        throw new Error(`chain_getHeader failed: ${JSON.stringify(response.error)}`);
    }
    const blockHex = response.result?.number;
    const height = parseInt(blockHex, 16);
    if (typeof blockHex !== 'string' || !/^0x[0-9a-f]+$/i.test(blockHex) || !Number.isSafeInteger(height)) {
        throw new Error(`chain_getHeader returned an invalid block number: ${String(blockHex)}`);
    }
    return height;
};

// Block until the indexer has processed the chain's best block as of this call. Tests wait
// a number of blocks after their transactions, and under `--sealing` a block comes every
// second, faster than the indexer catches up, so a block count no longer means the indexer
// has seen the event.
export const waitForIndexer = async (timeoutMs = 25_000): Promise<void> => {
    const deadline = Date.now() + timeoutMs;
    const requestTimeout = () => Math.max(1, Math.min(5_000, deadline - Date.now()));
    const target = await bestBlockNumber(requestTimeout());
    let processed = -1;
    while (Date.now() < deadline) {
        const response = await queryIndexedData('query { _metadata { lastProcessedHeight } }', requestTimeout());
        // eslint-disable-next-line no-underscore-dangle
        const height = response.data?._metadata?.lastProcessedHeight;
        processed = Number(height);
        const validHeight = typeof height === 'number' || (typeof height === 'string' && /^\d+$/.test(height));
        if (!validHeight || !Number.isSafeInteger(processed) || processed < 0) {
            throw new Error(`Indexer returned an invalid lastProcessedHeight: ${String(height)}`);
        }
        if (processed >= target) {
            return;
        }
        await sleep(Math.max(0, Math.min(250, deadline - Date.now())));
    }
    throw new Error(`indexer at block ${processed}, chain at ${target} after ${timeoutMs} ms`);
};

export const graphQLQuery = async (queryString: string) => {
    await waitForIndexer();
    return queryIndexedData(queryString);
};

// The chain's clock at its best block. Under `--sealing`, `waitEras` seals blocks ahead of
// time, so chain time runs ahead of the wall clock: "not in the future" means not later
// than the best block, not not later than `Date.now()`.
export const chainNow = async (api: ApiPromise): Promise<number> => (await api.query.timestamp.now()).toNumber();
