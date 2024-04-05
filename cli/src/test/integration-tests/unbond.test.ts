import { testIf, sleep } from '../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, BN, KeyringPair } from '../../lib';
import { getBalance } from '../../lib/balance';
import { parseAmount } from '../../commands/options';

describe('unbond', () => {
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

    describe('when NOT bonded', () => {
        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should error with validator not bonded message',
            () => {
                try {
                    CLI('unbond --amount 100');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain('Cannot perform action, validator is not bonded');
                }
            },
        );
    });

    describe('when ALREADY bonded', () => {
        beforeEach(() => {
            // bond before calling unbond
            const result = nonProxiedCli(`bond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        });

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient funds" message',
            () => {
                try {
                    CLI('unbond --amount 123');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain(
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
                    CLI('unbond --amount 123');
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
            'should be able to unbond',
            async () => {
                const zero = new BN(0);
                const oneHundred = parseAmount('100');
                const hundred23 = parseAmount('123');

                const oldBalance = await getBalance(caller.address, api);
                expect(oldBalance.unbonding.toString()).toBe(zero.toString());

                let result = CLI('unbond --amount 100');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);
                const newBalance = await getBalance(caller.address, api);
                expect(newBalance.unbonding.toString()).toBe(oneHundred.toString());

                // unbond again
                result = CLI('unbond --amount 23');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);
                const newerBalance = await getBalance(caller.address, api);
                expect(newerBalance.unbonding.toString()).toBe(hundred23.toString());
            },
            60_000,
        );
    });
});
