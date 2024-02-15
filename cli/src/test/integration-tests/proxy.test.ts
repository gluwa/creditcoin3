import {
    fundFromSudo,
    initAliceKeyring,
    ALICE_NODE_URL,
    BOB_NODE_URL,
    randomFundedAccount,
    randomTestAccount,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { parseAmount } from '../../commands/options';

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
        it('should execute without errors', () => {
            const result = CLI(`proxy add --proxy ${proxy.address} --type NonTransfer --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        }, 60_000);

        it('should error when caller does not have funds to pay fees', () => {
            // setup
            const caller2 = randomTestAccount();
            const cli = CLIBuilder({ CC_SECRET: caller2.secret });

            // test
            try {
                cli(`proxy add --proxy ${proxy.address} --type Staking`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Caller ${caller2.address} has insufficient funds to send the transaction`,
                );
            }
        }, 60_000);

        it('should error when caller already has configured a proxy', () => {
            // setup
            const result = CLI(`proxy add --proxy ${proxy.address} --type NonTransfer`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
            const proxy2 = randomTestAccount();

            // test
            try {
                CLI(`proxy add --proxy ${proxy2.address} --type Staking`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`ERROR: There is already an existing proxy set for ${caller.address}`);
            }
        }, 90_000);

        it('should error when trying to configure a proxy used by another delegate', async () => {
            // setup
            const caller2 = await randomFundedAccount(api, sudoSigner);
            const cli = CLIBuilder({ CC_SECRET: caller2.secret });
            const result = cli(`proxy add --proxy ${proxy.address} --type All`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // test
            try {
                CLI(`proxy add --proxy ${proxy.address} --type Staking`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(2);
                expect(error.stderr).toContain(
                    `ERROR: The proxy ${proxy.address} is already in use with another validator`,
                );
            }
        }, 90_000);
    });

    describe('proxy remove', () => {
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

        it('should error when caller does not have funds to pay fees', async () => {
            // setup
            const result = CLI(`proxy add --proxy ${proxy.address} --type All`);
            expect(result.exitCode).toEqual(0);
            await fundFromSudo(caller.address, parseAmount('1'));

            // test
            try {
                CLI(`proxy remove --proxy ${proxy.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(
                    `Caller ${caller.address} has insufficient funds to send the transaction`,
                );
            }
        }, 60_000);

        it('should error when no proxy defined', () => {
            // test
            try {
                CLI(`proxy remove --proxy ${proxy.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`ERROR: No proxies have been set for ${caller.address}`);
            }
        }, 60_000);

        it('should error when removing a non-proxy address', () => {
            // setup
            const result = CLI(`proxy add --proxy ${proxy.address} --type All`);
            expect(result.exitCode).toEqual(0);
            const proxy2 = randomTestAccount();

            // test
            try {
                CLI(`proxy remove --proxy ${proxy2.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`ERROR: ${proxy2.address} is not a proxy for ${caller.address}`);
            }
        }, 60_000);
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
