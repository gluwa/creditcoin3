import { testIf } from '../utils';
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
        let caller: any;

        // WARNING: caller is a local variable in each describe() block
        // b/c for some scenarios in the block above it changes beforeEach()
        // while here the entire setup is inside beforeAll() (b/c it takes a long time)
        beforeAll(async () => {
            // Create and fund the test and proxy account
            caller = await randomFundedAccount(api, sudoSigner);
            nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

            // bond before calling unbond
            let result = nonProxiedCli(`bond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            result = nonProxiedCli(`unbond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait for funds to become unlocked
            const unbondingPeriod: number = api.consts.staking.bondingDuration.toNumber();
            await waitEras(unbondingPeriod + 1, api, true);

            // configure proxy
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
        }, 1_200_000);

        afterAll(() => {
            tearDownProxy(nonProxiedCli, proxy);
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
            'should be able to withdraw',
            async () => {
                const zero = new BN(0);
                const hundred23 = parseAmount('123');

                const oldBalance = await getBalance(caller.address, api);
                expect(oldBalance.locked.toString()).toBe(hundred23.toString());

                const result = CLI('withdraw-unbonded');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 5 seconds for nodes to sync
                await new Promise((resolve) => setTimeout(resolve, 5000));
                const newBalance = await getBalance(caller.address, api);
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
    });
});
