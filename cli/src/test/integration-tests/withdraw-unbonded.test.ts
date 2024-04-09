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
import { getValidatorStatus, validatorStatusTable } from '../../lib/staking/validatorStatus';
import { parseAmount } from '../../commands/options';

async function nextUnbondingInMs(validatorAddress: string, api: ApiPromise) {
    const status = await validatorStatusTable(await getValidatorStatus(validatorAddress, api), api, false);
    const unbondingData = (status.at(-1) as string[]).at(-1) as string;

    if (unbondingData === 'None') {
        return BigInt(0);
    }

    // returns data in milliseconds
    return BigInt(unbondingData.split(' CTC in ').at(-1) as string);
}

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
            await sleep(5000);
            result = nonProxiedCli(`unbond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait for funds to become unlocked
            const unbondingPeriod: number = api.consts.staking.bondingDuration.toNumber();
            await waitEras(unbondingPeriod, api, true);

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
                await sleep(5000);
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

    describe('when partial unbond has been unlocked', () => {
        let caller: any;

        // WARNING: caller is a local variable in each describe() block
        // b/c for some scenarios in the block above it changes beforeEach()
        // while here the entire setup is inside beforeAll() (b/c it takes a long time)
        beforeAll(async () => {
            // Create and fund the test and proxy account
            caller = await randomFundedAccount(api, sudoSigner);
            nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

            // bond before calling unbond
            let result = nonProxiedCli(`bond --amount 500`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 1 era before unbonding to simulate a running chain
            // b/c unbonding in era 0 seems to give different results
            await waitEras(1, api, false);
            result = nonProxiedCli(`unbond --amount 123`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // begin unbonding era count-down
            const unbondingPeriod: number = api.consts.staking.bondingDuration.toNumber();
            const erasCountdownPromise = waitEras(unbondingPeriod, api, false);

            // before the unbonding period has expired check that reported remaining time (in ms) is decreasing.
            // WARNING: ONLY execute without a proxy b/c we're using `caller.address` directly
            if (process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no') {
                const blockTime = api.consts.babe.expectedBlockTime.toNumber();
                let oldUnbonding = await nextUnbondingInMs(caller.address, api);

                // note: 15 blocks * 2 epochs * 7 eras is 210 blocks !
                for (let i = 0; i < 200; i++) {
                    await sleep(blockTime);
                    const newUnbonding = await nextUnbondingInMs(caller.address, api);

                    // time always decreases towards zero
                    expect(oldUnbonding).toBeGreaterThanOrEqual(newUnbonding);

                    // diff between 2 consequtive queries is no more than 5 seconds
                    const difference = oldUnbonding - newUnbonding;
                    expect(difference).toBeLessThanOrEqual(blockTime);

                    oldUnbonding = newUnbonding;
                }
            }

            // configure proxy
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);

            // wait for funds to become unlocked
            await erasCountdownPromise;
        }, 1_500_000);

        afterAll(() => {
            tearDownProxy(nonProxiedCli, proxy);
        });

        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should be able to withdraw partially unlocked funds',
            async () => {
                const five00 = parseAmount('500');
                const three77 = parseAmount('377'); // 500 - 123

                const oldBalance = await getBalance(caller.address, api);
                expect(oldBalance.bonded.toString()).toBe(three77.toString());
                expect(oldBalance.locked.toString()).toBe(five00.toString());

                const result = CLI('withdraw-unbonded');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 5 seconds for nodes to sync
                await sleep(5000);
                const newBalance = await getBalance(caller.address, api);
                expect(newBalance.transferable > oldBalance.transferable).toBe(true);
                expect(newBalance.bonded.toString()).toBe(three77.toString());
                expect(newBalance.locked.toString()).toBe(three77.toString());

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
