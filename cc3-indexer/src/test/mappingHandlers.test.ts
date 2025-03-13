import { subqlTest } from '@subql/testing';

// See https://academy.subquery.network/build/testing.html

subqlTest(
    'Checkpoint reached test', // Test name
    142, // Block height to test at
    [], // Dependent entities
    [], // Expected entities
    'handleEventCheckpointReached', // handler name
);

subqlTest('Block attested event', 146, [], [], 'handleEventBlockAttested');
