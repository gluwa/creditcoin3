import { SubstrateProject, SubstrateRuntimeDatasource } from '@subql/types';
import { FrontierEvmDatasource } from "@subql/frontier-evm-processor";

import { proverDatasource, attestationDatasources, genesisDatasource } from './datasources';

import * as dotenv from 'dotenv';
import path from 'path';

const mode = process.env.NODE_ENV || 'production';

// Load the appropriate .env file
const dotenvPath = path.resolve(__dirname, `.env${mode !== 'production' ? `.${mode}` : ''}`);
dotenv.config({ path: dotenvPath });

const dataSources: (FrontierEvmDatasource | SubstrateRuntimeDatasource)[] = [];

if (process.env.DATASOURCE === 'prover') {
    dataSources.push(proverDatasource)
} else if (process.env.DATASOURCE === 'attestations') {
    dataSources.push(attestationDatasources)
    dataSources.push(genesisDatasource)
} else if (process.env.DATASOURCE === 'all-in-one') {
    dataSources.push(proverDatasource)
    dataSources.push(attestationDatasources)
    dataSources.push(genesisDatasource)
} else {
    throw new Error('DATASOURCE must be either prover or attestations')
}

// Can expand the Datasource processor types via the genreic param
const project: SubstrateProject<FrontierEvmDatasource> = {
    specVersion: '1.0.0',
    version: '0.0.1',
    name: 'polkadot-starter',
    description: 'This project can be used as a starting point for developing your SubQuery project',
    runner: {
        node: {
            name: '@subql/node',
            version: '>=3.0.1',
        },
        query: {
            name: '@subql/query',
            version: '*',
        },
    },
    schema: {
        file: './schema.graphql',
    },
    network: {
        /* The genesis hash of the network (hash of block 0) */
        chainId: process.env.CHAIN_ID!,
        /**
         * These endpoint(s) should be public non-pruned archive node
         * We recommend providing more than one endpoint for improved reliability, performance, and uptime
         * Public nodes may be rate limited, which can affect indexing speed
         * When developing your project we suggest getting a private API key
         * If you use a rate limited endpoint, adjust the --batch-size and --workers parameters
         * These settings can be found in your docker-compose.yaml, they will slow indexing but prevent your project being rate limited
         */
        endpoint: process.env.ENDPOINT!?.split(',') as string[] | string,
    },
    dataSources
};

// Must set default to the project instance
export default project;
