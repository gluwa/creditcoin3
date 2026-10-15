import type { Config } from '@jest/types';
import { junitReporters } from './junitReporters';

const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    reporters: junitReporters('cc3-indexer'),
    testTimeout: 30_000,
    globalSetup: './cc3-indexer-tests/globalSetup.ts',
    testSequencer: './alphabeticalSequencer.js',
};

export default config;
