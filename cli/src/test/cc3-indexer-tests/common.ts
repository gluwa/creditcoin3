import { createClient } from 'graphqurl';
import { ApiPromise } from '../../lib';

export const graphQLQuery = (queryString: string) => {
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

// The chain's clock at its best block. Under `--sealing`, `waitEras` seals blocks ahead of
// time, so chain time runs ahead of the wall clock: "not in the future" means not later
// than the best block, not not later than `Date.now()`.
export const chainNow = async (api: ApiPromise): Promise<number> => (await api.query.timestamp.now()).toNumber();
