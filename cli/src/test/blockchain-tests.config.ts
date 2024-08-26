import type { Config } from '@jest/types';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    testTimeout: 240000,
    globalSetup: process.env.BLOCKCHAIN_TESTS_GLOBAL_SETUP || './blockchainSetup.ts',
};

export default config;
