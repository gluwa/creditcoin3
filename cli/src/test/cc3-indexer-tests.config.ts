import type { Config } from '@jest/types';

const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    testTimeout: 30_000,
    globalSetup: './cc3-indexer-tests/globalSetup.ts',
};

export default config;
