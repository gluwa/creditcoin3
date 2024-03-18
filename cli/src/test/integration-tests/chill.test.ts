import { testIf } from '../utils';
import {
    initAliceKeyring,
    increaseValidatorCount,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';

describe('chill', () => {
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
        CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
    }, 90_000);

    afterEach(() => {
        tearDownProxy(nonProxiedCli, proxy);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when NOT validating', () => {
        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should error with validator not validating message',
            () => {
                try {
                    CLI('chill');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain('Cannot perform action, validator is not validating');
                }
            },
        );
    });

    describe('when ALREADY actively validating', () => {
        beforeAll(async () => {
            await increaseValidatorCount(api, sudoSigner);
        }, 30_000);

        beforeEach(async () => {
            // bond before calling validate
            let result = nonProxiedCli(`bond --amount 900`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // signal validation intention
            result = nonProxiedCli('validate --commission 90');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 10 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 10000));
            const status = await getValidatorStatus(caller.address, api);
            expect(status?.validating).toBe(true);
        }, 200_000);

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with account balance too low message',
            () => {
                try {
                    CLI('chill');
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
                    CLI('chill');
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
            'should cause validator to stop validating',
            async () => {
                const result = CLI('chill');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 5 seconds for nodes to sync
                await new Promise((resolve) => setTimeout(resolve, 5000));

                const status = await getValidatorStatus(caller.address, api);
                expect(status?.validating).toBe(false);
            },
            60_000,
        );
    });
});
