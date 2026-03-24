import type { Config } from '@jest/types';
const config: Config.InitialOptions = {
    preset: 'ts-jest',
    testEnvironment: 'node',
    globalSetup: './archiverSetup.ts',
};

export default config;
