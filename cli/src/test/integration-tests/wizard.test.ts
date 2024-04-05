import {
    initAliceKeyring,
    increaseValidatorCount,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
    setMinBondConfig,
} from './helpers';
import { testIf, sleep } from '../utils';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

describe('wizard', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let wrongProxy: any;
    let sudoSigner: KeyringPair;
    let CLI: any;
    let nonProxiedCli: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        sudoSigner = initAliceKeyring();
        await increaseValidatorCount(api, sudoSigner);
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

        proxy = await randomFundedAccount(api, sudoSigner);
        wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
    }, 90_000);

    afterEach(async () => {
        tearDownProxy(nonProxiedCli, proxy);

        // set default min bond config to 0
        await setMinBondConfig(api, 0);
    }, 90_000);

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                CLI('wizard --amount 900');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(
                    `Caller ${proxy.address} has insufficient funds to send the transaction`,
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
                CLI('wizard --amount 900');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(`Proxy ${wrongProxy.address} is not valid for ${caller.address}`);
            }
        },
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'new validator should appear as waiting after running',
        async () => {
            const result = CLI('wizard --amount 900');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await sleep(5000);

            const validatorStatus = await getValidatorStatus(caller.address, api);
            expect(validatorStatus?.waiting).toBe(true);
        },
        120_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when specified amount < MinValidatorBond',
        async () => {
            // set min bond amount to 5000
            const minValidatorBond = 5000;
            // set staking config min bond amount
            await setMinBondConfig(api, minValidatorBond);

            try {
                CLI('wizard --amount 900');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Amount to bond must be at least: ${minValidatorBond} (min validator bond amount)`,
                );
            }
        },
        120_000,
    );
});
