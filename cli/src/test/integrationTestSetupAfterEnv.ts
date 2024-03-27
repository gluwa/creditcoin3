import { expectIsFinalizing } from './utils';

global.beforeEach(async () => {
    // short-circuit test execution if the chain has stopped finalizing
    await expectIsFinalizing();
});
