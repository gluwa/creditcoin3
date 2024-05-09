import { expectIsFinalizing, startAliceAndBob, killCreditcoinNodes } from './utils';

global.beforeAll(async () => {
    // alternatively we can duplicate utils.describeIf() and place this
    // setup/teardown code in there. But make sure to have a separate function
    // which is only intended to be used at the outer-most layer
    await startAliceAndBob();
}, 20_000);

global.afterAll(() => {
    killCreditcoinNodes();
});

global.beforeEach(async () => {
    // short-circuit test execution if the chain has stopped finalizing
    await expectIsFinalizing();
}, 10_000);
