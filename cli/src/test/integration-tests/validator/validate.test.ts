import { testIf, try_catch_else_finally, sleep } from '../../utils';
import {
    initAliceKeyring,
    increaseValidatorCount,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
    setMinBondConfig,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { getValidatorStatus } from '../../../lib/staking/validatorStatus';

describe('validate', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let sudoSigner: KeyringPair;
    let CLI: any;
    let nonProxiedCli: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

        proxy = await randomFundedAccount(api, sudoSigner);
        const wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(api, nonProxiedCli, caller, proxy, wrongProxy);
    }, 90_000);

    afterEach(async () => {
        tearDownProxy(nonProxiedCli, proxy);

        // set default min bond config to 0
        await setMinBondConfig(api, new BN(0));
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when NOT bonded', () => {
        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should error with staking.NotController message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI('validate');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stdout).toContain('staking.NotController: Not a controller account.');
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
        );
    });

    describe('when ALREADY bonded', () => {
        beforeAll(async () => {
            await increaseValidatorCount(api, sudoSigner);
        }, 30_000);

        beforeEach(() => {
            // bond before calling validate
            const result = nonProxiedCli(`bond --amount 900`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        });

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient-funds" message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI('validate');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stderr).toContain(
                            `Caller ${proxy.address} has insufficient funds to send the transaction`,
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
            60_000,
        );

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
            'should error with proxy.NotProxy message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI('validate');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stdout).toContain(
                            'Transaction failed with error: "proxy.NotProxy: Sender is not a proxy of the account to be proxied."',
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
        );

        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should become a waiting validator',
            async () => {
                const result = CLI('validate --commission 90');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);

                const status = await getValidatorStatus(caller.address, api);
                expect(status?.waiting).toBe(true);
            },
            60_000,
        );
    });

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when current bond < MinValidatorBond',
        async () => {
            // Bond 900
            CLI('bond --amount 900');

            // set min bond amount to 1000 *AFTER* bonding to circumvent the
            // check inside the `bond` command! Polkadot-SDK allows you to bond any amount
            const minValidatorBond = MICROUNITS_PER_CTC.mul(new BN(1000));
            await setMinBondConfig(api, minValidatorBond);

            try_catch_else_finally(
                () => {
                    // try validate now
                    CLI('validate --commission 90');
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    const expectedMin = minValidatorBond.div(MICROUNITS_PER_CTC).toString();
                    expect(error.stderr).toContain(
                        `Amount to bond must be at least: ${expectedMin} CTC (min validator bond amount)`,
                    );
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        60_000,
    );
});
