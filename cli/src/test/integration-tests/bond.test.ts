import { testIf, sleep } from '../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
    setMinBondConfig,
} from './helpers';
import { newApi, ApiPromise, BN, KeyringPair } from '../../lib';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';

describe('bond', () => {
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

    afterEach(async () => {
        tearDownProxy(nonProxiedCli, proxy);

        // set default min bond config to 0
        await setMinBondConfig(api, 0);
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                CLI('bond --amount 111');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Caller ${proxy.address} has insufficient funds to send the transaction`,
                );
            }
        },
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
        'should error with proxy.NotProxy message',
        () => {
            try {
                CLI('bond --amount 222');
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
        'should bond specified amount + extra',
        async () => {
            const zero = new BN(0);
            const three33 = parseAmount('333');
            const four44 = parseAmount('444');

            const oldBalance = await getBalance(caller.address, api);
            expect(oldBalance.bonded.toString()).toBe(zero.toString());
            expect(oldBalance.locked.toString()).toBe(zero.toString());

            let result = CLI('bond --amount 333');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
            const newBalance = await getBalance(caller.address, api);
            expect(newBalance.bonded.toString()).toBe(three33.toString());
            expect(newBalance.locked.toString()).toBe(three33.toString());

            // bond extra
            result = CLI('bond --amount 111 --extra');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
            const newerBalance = await getBalance(caller.address, api);
            expect(newerBalance.bonded.toString()).toBe(four44.toString());
            expect(newerBalance.locked.toString()).toBe(four44.toString());
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should fail when already bonded',
        () => {
            // setup
            const result = CLI('bond --amount 333');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            try {
                // call bond again w/o the --extra flag
                CLI('bond --amount 111');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain('staking.AlreadyBonded: Stash is already bonded.');
            }
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when specified amount < MinValidatorBond',
        async () => {
            const minValidatorBond = 100;
            // set staking config min bond amount
            await setMinBondConfig(api, minValidatorBond);

            const zero = new BN(0);
            const balance = await getBalance(caller.address, api);
            expect(balance.bonded.toString()).toBe(zero.toString());
            expect(balance.locked.toString()).toBe(zero.toString());

            try {
                CLI('bond --amount 50');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Amount to bond must be at least: ${minValidatorBond} (min validator bond amount)`,
                );
            }
        },
        90_000,
    );
});
