import 'jest-expect-message';
import type { Config } from '@jest/types';

const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    testTimeout: 30_000,
    setupFilesAfterEnv: ['jest-expect-message', './integrationTestSetupAfterEnv.ts'],
};

export default config;
