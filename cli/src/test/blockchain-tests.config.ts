import type { Config } from '@jest/types';
const config: Config.InitialOptions = {
    preset: 'ts-jest/presets/default-esm',
    testEnvironment: 'node',
    testTimeout: 240000,
    globalSetup: process.env.BLOCKCHAIN_TESTS_GLOBAL_SETUP || './blockchainSetup.ts',
    extensionsToTreatAsEsm: ['.ts'],
    transform: {
        // eslint-disable-next-line @typescript-eslint/naming-convention
        '^.+\\.tsx?$': ['ts-jest', { useESM: true, tsconfig: 'tsconfig.test.json' }],
    },
    transformIgnorePatterns: [
        'node_modules/(?!(execa|@sindresorhus/merge-streams|figures|get-stream|human-signals|is-plain-obj|is-stream|npm-run-path|path-key|pretty-ms|signal-exit|strip-final-newline|which-command|yoctocolors|unicorn-magic)/)',
    ],
};

export default config;
