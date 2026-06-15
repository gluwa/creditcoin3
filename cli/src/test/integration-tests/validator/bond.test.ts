import { testIf, try_catch_else_finally, sleep } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
    setMinBondConfig,
    fundFromSudo,
} from '../helpers';
import { newApi, ApiPromise, BN, KeyringPair, MICROUNITS_PER_CTC } from '../../../lib';
import { getBalance } from '../../../lib/balance';
import { parseAmount } from '../../../commands/options';

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

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try_catch_else_finally(
                () => {
                    CLI('bond --amount 111');
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
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
        'should error with proxy.NotProxy message',
        () => {
            try_catch_else_finally(
                () => {
                    CLI('bond --amount 222');
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

            try_catch_else_finally(
                () => {
                    // call bond again w/o the --extra flag
                    CLI('bond --amount 111');
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain('staking.AlreadyBonded: Stash is already bonded.');
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when specified amount < MinValidatorBond',
        async () => {
            const minValidatorBond = MICROUNITS_PER_CTC.mul(new BN(100));
            // set staking config min bond amount
            await setMinBondConfig(api, minValidatorBond);

            const zero = new BN(0);
            const balance = await getBalance(caller.address, api);
            expect(balance.bonded.toString()).toBe(zero.toString());
            expect(balance.locked.toString()).toBe(zero.toString());

            try_catch_else_finally(
                () => {
                    CLI('bond --amount 50');
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
        120_000,
    );

    // Reproduces devnet bug: bonding 1100 CTC when minValidatorBond is 999 CTC
    // This SHOULD succeed after the fix since 1100 > 999, but it used to fail due to
    // BN string comparison bug in `hasBondedEnough` fn
    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should successfully bond 1100 CTC when minValidatorBond is 999 CTC',
        async () => {
            // Set minValidatorBond to 999 CTC (in raw units: 999 * 10^18)
            const minValidatorBondRaw = '999000000000000000000';
            await setMinBondConfig(api, minValidatorBondRaw);

            // Fund account with enough CTC (default is 1000, we need 1100+ for bond + fees)
            await fundFromSudo(api, caller.address, parseAmount('2000'));

            // Verify account starts with zero bonded
            const zero = new BN(0);
            const balance = await getBalance(caller.address, api);
            expect(balance.bonded.toString()).toBe(zero.toString());

            // Bond 1100 CTC - this SHOULD succeed since 1100 > 999
            const result = CLI('bond --amount 1100');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // Verify the bond was successful
            await sleep(2000);
            const newBalance = await getBalance(caller.address, api);
            const expectedBonded = parseAmount('1100');
            expect(newBalance.bonded.toString()).toBe(expectedBonded.toString());
        },
        90_000,
    );
});
