import { testIf, sleep } from '../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    waitEras,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, BN, KeyringPair } from '../../lib';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';

describe('withdraw-unbonded', () => {
    let api: ApiPromise;
    let proxy: any;
    let sudoSigner: KeyringPair;
    let CLI: any;
    let nonProxiedCli: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when there are NO unlocked funds', () => {
        let caller: any;

        beforeEach(async () => {
            // Create and fund the test and proxy account
            caller = await randomFundedAccount(api, sudoSigner);
            nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });
        }, 90_000);

        testIf(
            process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
            'should error with no unlocked funds message',
            () => {
                try {
                    // note that we're not even bonded
                    nonProxiedCli('withdraw-unbonded');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain('Cannot perform action, there are no unlocked funds to withdraw');
                }
            },
        );
    });

    describe('when funds have been unlocked', () => {
        let callerFullUnbond: any;
        let callerPartialUnbond: any;
        let nonProxiedCliFullUnbond: any;
        let nonProxiedCliPartialUnbond: any;

        // WARNING: caller is a local variable in each describe() block
        // b/c for some scenarios in the block above it changes beforeEach()
        // while here the entire setup is inside beforeAll() (b/c it takes a long time)
        beforeAll(async () => {
            // Create and fund the test and proxy account
            callerFullUnbond = await randomFundedAccount(api, sudoSigner);
            nonProxiedCliFullUnbond = CLIBuilder({ CC_SECRET: callerFullUnbond.secret });

            // bond before calling unbond
            let result = nonProxiedCliFullUnbond(`bond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            callerPartialUnbond = await randomFundedAccount(api, sudoSigner);
            nonProxiedCliPartialUnbond = CLIBuilder({ CC_SECRET: callerPartialUnbond.secret });

            // bond before calling unbond
            result = nonProxiedCliPartialUnbond(`bond --amount 500`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);

            // Full Unbond
            result = nonProxiedCliFullUnbond(`unbond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // Partial Unbond
            result = nonProxiedCliPartialUnbond(`unbond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // begin unbonding era count-down
            const unbondingPeriod: number = api.consts.staking.bondingDuration.toNumber();
            const erasCountdownPromise = waitEras(unbondingPeriod, api);

            // configure proxy - used only for Full Unbond scenarios
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(nonProxiedCliFullUnbond, callerFullUnbond, proxy, wrongProxy);

            // wait for funds to become unlocked
            await erasCountdownPromise;
        }, 1_200_000);

        afterAll(() => {
            tearDownProxy(nonProxiedCliFullUnbond, proxy);
        });

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with account balance too low message',
            () => {
                try {
                    CLI('withdraw-unbonded');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain(
                        'Invalid Transaction: Inability to pay some fees , e.g. account balance too low',
                    );
                }
            },
            60_000,
        );

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
            'should error with proxy.NotProxy message',
            () => {
                try {
                    CLI('withdraw-unbonded');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(
                        'Transaction failed with error: "proxy.NotProxy: Sender is not a proxy of the account to be proxied."',
                    );
                }
            },
        );

        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should be able to withdraw fully unbonded amount',
            async () => {
                const zero = new BN(0);
                const hundred23 = parseAmount('123');

                const oldBalance = await getBalance(callerFullUnbond.address, api);
                expect(oldBalance.locked.toString()).toBe(hundred23.toString());

                const result = CLI('withdraw-unbonded');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);
                const newBalance = await getBalance(callerFullUnbond.address, api);
                expect(newBalance.locked.toString()).toBe(zero.toString());

                // try to withdraw again - should fail
                try {
                    CLI('withdraw-unbonded');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain('Cannot perform action, there are no unlocked funds to withdraw');
                }
            },
            90_000,
        );

        testIf(
            process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
            'should be able to withdraw partially unbonded amount',
            async () => {
                const five00 = parseAmount('500');
                const three77 = parseAmount('377'); // 500 - 123

                const oldBalance = await getBalance(callerPartialUnbond.address, api);
                expect(oldBalance.bonded.toString()).toBe(three77.toString());
                expect(oldBalance.locked.toString()).toBe(five00.toString());

                const result = nonProxiedCliPartialUnbond('withdraw-unbonded');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);
                const newBalance = await getBalance(callerPartialUnbond.address, api);
                expect(newBalance.transferable > oldBalance.transferable).toBe(true);
                expect(newBalance.bonded.toString()).toBe(three77.toString());
                expect(newBalance.locked.toString()).toBe(three77.toString());

                // try to withdraw again - should fail
                try {
                    nonProxiedCliPartialUnbond('withdraw-unbonded');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain('Cannot perform action, there are no unlocked funds to withdraw');
                }
            },
            90_000,
        );
    });
});
