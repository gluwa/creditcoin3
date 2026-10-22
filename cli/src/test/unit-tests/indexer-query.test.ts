import { createClient } from 'graphqurl';
import { graphQLQuery, indexedRecordQuery } from '../cc3-indexer-tests/common';

jest.mock('graphqurl', () => ({ createClient: jest.fn() }));
jest.mock('../utils', () => ({
    sleep: (ms: number) => new Promise((resolve) => setTimeout(resolve, ms)),
}));

describe('indexer query synchronization', () => {
    const originalGraphQLUrl = (global as any).GRAPHQL_URL;
    const originalApiUrl = (global as any).CREDITCOIN_API_URL;
    const query = jest.fn();
    let header: jest.SpyInstance;
    let bestHeight: number;
    let processedHeight: number;
    const metadataQuery = 'query { _metadata { lastProcessedHeight } }';
    const listQuery = 'query { events { nodes { id } } }';
    const records = Array.from({ length: 10 }, (_, index) => ({ id: `100-${index}` }));
    const recordQueries = new Map(records.map((record) => [`query { event(id: "${record.id}") { id } }`, record]));

    beforeEach(() => {
        jest.useFakeTimers();
        bestHeight = 100;
        processedHeight = 99;
        (global as any).GRAPHQL_URL = 'http://indexer.test';
        (global as any).CREDITCOIN_API_URL = 'ws://node.test';
        header = jest.spyOn(global, 'fetch').mockImplementation(() => {
            return Promise.resolve({
                // eslint-disable-next-line id-denylist
                json: () => Promise.resolve({ result: { number: `0x${bestHeight.toString(16)}` } }),
            } as Response);
        });
        jest.mocked(createClient).mockReturnValue({ query } as unknown as ReturnType<typeof createClient>);
        query.mockReset().mockImplementation(({ query: requested }: { query: string }) => {
            if (requested === metadataQuery) {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                return Promise.resolve({ data: { _metadata: { lastProcessedHeight: processedHeight } } });
            }
            if (requested === listQuery) {
                return Promise.resolve({ data: { events: { nodes: records } } });
            }
            const record = recordQueries.get(requested);
            if (record) {
                return Promise.resolve({ data: { event: record } });
            }
            throw new Error(`unexpected query: ${requested}`);
        });
    });

    afterEach(() => {
        jest.restoreAllMocks();
        jest.useRealTimers();
        (global as any).GRAPHQL_URL = originalGraphQLUrl;
        (global as any).CREDITCOIN_API_URL = originalApiUrl;
    });

    test('same-record readbacks do not chase the tip, but the next normal query synchronizes again', async () => {
        const listed = graphQLQuery(listQuery);
        await jest.advanceTimersByTimeAsync(0);
        expect(query).not.toHaveBeenCalledWith({ query: listQuery });

        // The original barrier must only wait for its captured height, even as the chain advances.
        bestHeight = 200;
        processedHeight = 100;
        await jest.advanceTimersByTimeAsync(250);
        const response = await listed;
        expect(response.data.events.nodes).toEqual(records);

        // Reproduce the ten per-ID endpoint checks while the indexer remains behind the new tip.
        for (const record of response.data.events.nodes) {
            const readback = await indexedRecordQuery(`query { event(id: "${record.id}") { id } }`);
            expect(readback.data.event).toEqual(record);
        }
        expect(header).toHaveBeenCalledTimes(1);
        expect(query.mock.calls.filter(([request]) => request.query === metadataQuery)).toHaveLength(2);

        query.mockClear();
        const fresh = graphQLQuery(listQuery);
        await jest.advanceTimersByTimeAsync(0);
        expect(header).toHaveBeenCalledTimes(2);
        expect(query).toHaveBeenCalledWith({ query: metadataQuery });
        expect(query).not.toHaveBeenCalledWith({ query: listQuery });

        processedHeight = 200;
        await jest.advanceTimersByTimeAsync(250);
        expect((await fresh).data.events.nodes).toEqual(records);
    });

    test('a normal query still fails if the indexer never reaches its captured height', async () => {
        bestHeight = 200;
        processedHeight = 100;
        const result = graphQLQuery(listQuery);
        const rejected = expect(result).rejects.toThrow('indexer at block 100, chain at 200 after 25000 ms');
        await jest.advanceTimersByTimeAsync(25_000);
        await rejected;
        expect(header).toHaveBeenCalledTimes(1);
        expect(query).not.toHaveBeenCalledWith({ query: listQuery });
    });
});
