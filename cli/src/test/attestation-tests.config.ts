import type { Config } from '@jest/types';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    globalSetup: './attestationSetup.ts',
};

export default config;
