import type { Config } from '@jest/types';
import { junitReporters } from './junitReporters';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    reporters: junitReporters('blockchain'),
    testTimeout: 240000,
    globalSetup: process.env.BLOCKCHAIN_TESTS_GLOBAL_SETUP || './blockchainSetup.ts',
};

export default config;
