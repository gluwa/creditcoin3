import { startAliceAndBob, killCreditcoinNodes } from './utils';

global.beforeAll(async () => {
    // alternatively we can duplicate utils.describeIf() and place this
    // setup/teardown code in there. But make sure to have a separate function
    // which is only intended to be used at the outer-most layer
    await startAliceAndBob();
}, 10_000);

global.afterAll(() => {
    killCreditcoinNodes();
});
