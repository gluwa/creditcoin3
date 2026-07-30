import 'jest-expect-message';
import type { Config } from '@jest/types';

const config: Config.InitialOptions = {
    preset: 'ts-jest/presets/default-esm',
    testEnvironment: 'node',
    testTimeout: 30_000,
    setupFilesAfterEnv: ['jest-expect-message', './integrationTestSetupAfterEnv.ts'],
    extensionsToTreatAsEsm: ['.ts'],
    transform: {
        // eslint-disable-next-line @typescript-eslint/naming-convention
        '^.+\\.tsx?$': ['ts-jest', { useESM: true, tsconfig: 'tsconfig.test.json' }],
    },
    // execa >=6 is pure ESM; let ts-jest transpile it (and its ESM-only deps)
    // instead of ignoring node_modules, otherwise jest's runtime hits
    // "Cannot use import statement outside a module" on execa's ESM entrypoint.
    transformIgnorePatterns: [
        'node_modules/(?!(execa|@sindresorhus/merge-streams|figures|get-stream|human-signals|is-plain-obj|is-stream|npm-run-path|path-key|pretty-ms|signal-exit|strip-final-newline|which-command|yoctocolors|unicorn-magic)/)',
    ],
};

export default config;
