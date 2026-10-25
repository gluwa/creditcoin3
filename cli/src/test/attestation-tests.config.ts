import type { Config } from '@jest/types';
import { junitReporters } from './junitReporters';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    reporters: junitReporters('attestation'),
    globalSetup: './attestationSetup.ts',
};

export default config;
