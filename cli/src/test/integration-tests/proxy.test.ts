import {
    initAliceKeyring,
    ALICE_NODE_URL,
    BOB_NODE_URL,
    randomFundedAccount,
    randomTestAccount,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

describe('Proxy functionality', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let sudoSigner: KeyringPair;
    let CLI: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
        proxy = await randomFundedAccount(api, sudoSigner);

        // Create a CLICmd instance with a properly configured environment
        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({ CC_SECRET: caller.secret, CC_PROXY_SECRET: proxy.secret });
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('proxy list', () => {
        it('should display no proxies when none are configured', () => {
            const result = CLI('proxy list');

            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('No proxies for address');
        }, 30_000);

        it('should display proxies which are configured', () => {
            const proxy2 = randomTestAccount();

            // setup
            let result = CLI(`proxy add --proxy ${proxy.address} --type Staking`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // test
            result = CLI(`proxy list --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain(proxy.address); // The proxy address should be listed
            expect(result.stdout).toContain('Staking'); // The type should be correctly listed as 'Staking'
            expect(result.stdout).not.toContain(proxy2.address);
        }, 60_000);
    });

    describe('proxy add', () => {
        // todo: add error handling scenarios here

        it('should execute without errors', () => {
            const result = CLI(`proxy add --proxy ${proxy.address} --type NonTransfer --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        }, 60_000);
    });

    describe('proxy remove', () => {
        // todo: add error handling scenarios here

        it('should remove a configured proxy without errors', () => {
            // setup
            let result = CLI(`proxy add --proxy ${proxy.address} --type All`);
            expect(result.exitCode).toEqual(0);

            // test
            result = CLI(`proxy remove --proxy ${proxy.address} --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // verify
            result = CLI('proxy list');
            expect(result.stdout).toContain('No proxies for address');
        }, 90_000);
    });

    // todo: CSUB-1025
    it('Can successfully bond and unbond with a proxy account', () => {
        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type Staking`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Successfully bond for the first time
        const test1Res = CLI(`bond --amount 1 --use-proxy ${caller.address}`);
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');

        // Test #2. Attempt to bond extra without specifying the extra command
        // TODO This should fail but the signSendAndWatch function needs to be updated
        const test2Res = CLI(`bond --amount 1 --use-proxy ${caller.address}`);
        expect(test2Res.exitCode).toEqual(0);

        // Test #3. Successfully bond extra using the proxy
        const test3Res = CLI(`bond --amount 1 -x --use-proxy ${caller.address}`);
        expect(test3Res.exitCode).toEqual(0);
        expect(test3Res.stdout).toContain('Transaction included at block');

        // Test #4. Successfully unbond extra using the proxy
        const test4Res = CLI(`unbond --amount 1 --use-proxy ${caller.address}`);
        expect(test4Res.exitCode).toEqual(0);
        expect(test4Res.stdout).toContain('Transaction included at block');
    }, 60000);

    // todo: CSUB-1025
    it('Can successfully validate and chill with a proxy account', () => {
        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type Staking`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Successfully bond for the first time
        const test1Res = CLI(`bond --use-proxy ${caller.address} --amount 100`);
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');

        // Test #2. Successfully bond for the first time
        const test2Res = CLI(`validate --use-proxy ${caller.address}`);
        expect(test2Res.exitCode).toEqual(0);
        expect(test2Res.stdout).toContain('Transaction included at block');

        // Test #3. Attempt to bond extra without specifying the extra command
        // TODO This should fail but the signSendAndWatch function needs to be updated
        const test3Res = CLI(`chill --use-proxy ${caller.address}`);
        expect(test3Res.exitCode).toEqual(0);
        expect(test3Res.stdout).toContain('Transaction included at block');
    }, 360_000);

    // todo: CSUB-1025
    it('Can successfully send funds with a proxy', () => {
        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type All`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Send money to Alice
        const test1Res = CLI(
            `send --amount 1 --use-proxy ${caller.address} --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY`,
        );
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');
    }, 360_000);
});
