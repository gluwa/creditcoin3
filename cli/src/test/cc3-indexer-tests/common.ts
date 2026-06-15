import { createClient } from 'graphqurl';

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
