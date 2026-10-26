import { graphQLQuery, queryIndexedData, waitForIndexer } from '../cc3-indexer-tests/common';

// The JSON-RPC header field is named "number".
// eslint-disable-next-line id-denylist
const head = (blockHex: string) => Response.json({ result: { number: blockHex } });
const indexed = (height: unknown) =>
    // eslint-disable-next-line no-underscore-dangle, @typescript-eslint/naming-convention
    Response.json({ data: { _metadata: { lastProcessedHeight: height } } });

describe('indexer query synchronization', () => {
    let fetchMock: jest.SpiedFunction<typeof fetch>;

    beforeEach(() => {
        (global as any).CREDITCOIN_API_URL = 'ws://chain.example:9944';
        (global as any).GRAPHQL_URL = 'http://indexer.example:3000';
        fetchMock = jest.spyOn(global, 'fetch');
    });

    afterEach(() => {
        jest.restoreAllMocks();
    });

    test('entity lookups do not chase newer heads after the initial query is synchronized', async () => {
        fetchMock
            .mockResolvedValueOnce(head('0x64'))
            .mockResolvedValueOnce(indexed(100))
            .mockResolvedValueOnce(Response.json({ data: { entities: [{ id: 'known' }] } }))
            .mockImplementation(() => Promise.resolve(Response.json({ data: { entity: { id: 'known' } } })));

        const list = await graphQLQuery('query { entities { id } }');
        for (let i = 0; i < 10; i++) {
            const result = await queryIndexedData(`query { entity(id: "${list.data.entities[0].id}") { id } }`);
            expect(result.data.entity.id).toEqual('known');
        }

        const bodies = fetchMock.mock.calls.map(([, options]) => JSON.parse(options?.body as string));
        expect(bodies.filter((body) => body.method === 'chain_getHeader')).toHaveLength(1);
        expect(bodies.filter((body) => body.query?.includes('_metadata'))).toHaveLength(1);
        expect(bodies.filter((body) => body.query?.includes('entity(id:'))).toHaveLength(10);
    });

    test('a later chain operation can request fresh synchronization in the same test', async () => {
        fetchMock
            .mockResolvedValueOnce(head('0x64'))
            .mockResolvedValueOnce(indexed(100))
            .mockResolvedValueOnce(Response.json({ data: { value: 'before' } }))
            .mockResolvedValueOnce(head('0x65'))
            .mockResolvedValueOnce(indexed(101))
            .mockResolvedValueOnce(Response.json({ data: { value: 'after' } }));

        expect((await graphQLQuery('query { value }')).data.value).toEqual('before');
        expect((await graphQLQuery('query { value }')).data.value).toEqual('after');
        expect(fetchMock).toHaveBeenCalledTimes(6);
    });

    test('polling waits for the captured block rather than reading a moving head', async () => {
        fetchMock
            .mockResolvedValueOnce(head('0x64'))
            .mockResolvedValueOnce(indexed('99'))
            .mockResolvedValueOnce(indexed('100'));

        await waitForIndexer();
        expect(fetchMock).toHaveBeenCalledTimes(3);
        expect(fetchMock.mock.calls[0][0]).toEqual('http://chain.example:9944');
    });

    test('a stalled indexer reports its processed and target heights', async () => {
        fetchMock.mockResolvedValueOnce(head('0x64')).mockImplementation(() => Promise.resolve(indexed(90)));

        await expect(waitForIndexer(20)).rejects.toThrow('indexer at block 90, chain at 100 after 20 ms');
    });

    test.each([null, '', false, 'invalid', -1])(
        'invalid indexer height %p cannot satisfy readiness',
        async (height) => {
            fetchMock.mockResolvedValueOnce(head('0x0')).mockResolvedValueOnce(indexed(height));

            await expect(waitForIndexer()).rejects.toThrow('Indexer returned an invalid lastProcessedHeight');
        },
    );

    test('JSON-RPC errors fail before querying the indexer', async () => {
        fetchMock.mockResolvedValueOnce(Response.json({ error: { code: -32000, message: 'node unavailable' } }));

        await expect(waitForIndexer()).rejects.toThrow('chain_getHeader failed');
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    test('a malformed chain header fails before querying the indexer', async () => {
        fetchMock.mockResolvedValueOnce(head('invalid'));

        await expect(waitForIndexer()).rejects.toThrow('chain_getHeader returned an invalid block number');
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    test('HTTP errors are reported instead of being treated as indexed data', async () => {
        fetchMock.mockResolvedValueOnce(new Response('unavailable', { status: 503 }));

        await expect(queryIndexedData('query { value }')).rejects.toThrow('HTTP 503');
    });

    test('GraphQL errors fail even if the response includes partial data', async () => {
        fetchMock.mockResolvedValueOnce(
            Response.json({ data: { value: null }, errors: [{ message: 'query failed' }] }),
        );

        await expect(queryIndexedData('query { value }')).rejects.toThrow('GraphQL query failed');
    });

    test.each(['chain', 'indexer'])('a hanging %s request is aborted within the readiness budget', async (endpoint) => {
        if (endpoint === 'indexer') {
            fetchMock.mockResolvedValueOnce(head('0x64'));
        }
        fetchMock.mockImplementationOnce(
            (_input, options) =>
                new Promise((_resolve, reject) => {
                    const signal = options?.signal;
                    signal?.addEventListener(
                        'abort',
                        () => reject(signal.reason instanceof Error ? signal.reason : new Error('request timeout')),
                        { once: true },
                    );
                }),
        );

        await expect(waitForIndexer(20)).rejects.toThrow(/timeout/i);
        expect(fetchMock.mock.lastCall?.[1]?.signal?.aborted).toBe(true);
    });
});
