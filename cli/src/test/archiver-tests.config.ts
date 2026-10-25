import type { Config } from '@jest/types';
import { junitReporters } from './junitReporters';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    reporters: junitReporters('archiver'),
    globalSetup: './archiverSetup.ts',
};

export default config;
