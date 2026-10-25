import type { Config } from '@jest/types';

// Keep the console output and also write a JUnit XML report per suite, so CI can track
// per-test durations and failures over time. CI uploads `reports/junit/` (relative to cli/).
export const junitReporters = (suite: string): Config.InitialOptions['reporters'] => [
    'default',
    [
        'jest-junit',
        {
            outputDirectory: 'reports/junit',
            outputName: `${suite}.xml`,
            suiteNameTemplate: '{filepath}',
            addFileAttribute: 'true',
            reportTestSuiteErrors: 'true',
        },
    ],
];
