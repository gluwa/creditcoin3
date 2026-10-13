import 'jest-expect-message';
import type { Config } from '@jest/types';
import { junitReporters } from './junitReporters';

const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    reporters: junitReporters('integration'),
    testTimeout: 30_000,
    setupFilesAfterEnv: ['jest-expect-message', './integrationTestSetupAfterEnv.ts'],
};

export default config;
