import { testIf, try_catch_else_finally, sleep } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    waitEras,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, BN, KeyringPair } from '../../../lib';
import { getBalance } from '../../../lib/balance';
import { getValidatorStatus } from '../../../lib/staking/validatorStatus';
import { parseAmount } from '../../../commands/options';

async function nextUnbondingInMs(validatorAddress: string, api: ApiPromise): Promise<bigint> {
    const status = await getValidatorStatus(validatorAddress, api);
    if (!status?.nextUnlocking.length) {
        return 0n;
    }
    return BigInt(status.nextUnlocking[0].millis);
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
                try_catch_else_finally(
                    () => {
                        // note that we're not even bonded
                        nonProxiedCli('withdraw-unbonded');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stderr).toContain(
                            'Cannot perform action, there are no unlocked funds to withdraw',
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
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

            // wait 1 era before unbonding to simulate a running chain
            // b/c unbonding in era 0 seems to give different results
            await waitEras(1, api);

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

            // before the unbonding period has expired check that reported remaining time (in ms) is decreasing.
            // WARNING: ONLY execute without a proxy b/c we're using `caller.address` directly
            if (process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no') {
                const blockTime = api.consts.babe.expectedBlockTime.toNumber();
                const eraLengthBlocks = (await api.derive.session.progress()).eraLength.toNumber();
                // Per-block samples are ~[blockTime, 2×blockTime]; an era rollover drops ~eraLength blocks.
                const maxDecreaseMs = eraLengthBlocks * blockTime + blockTime * 2;
                let oldUnbonding = await nextUnbondingInMs(callerFullUnbond.address, api);

                // Sample the countdown for a few blocks, including at least one era boundary while
                // `waitEras(bondingDuration)` runs in parallel.
                const maxIterations = 30;
                for (let i = 0; i < maxIterations; i++) {
                    const errMsg = `Failed on iteration #${i}/${maxIterations}`;

                    // note: we sleep exactly blockTime but the other calls still take some time
                    // and it's possible that at some iterations the query spans 2 blocks
                    await sleep(blockTime);
                    const newUnbonding = await nextUnbondingInMs(callerFullUnbond.address, api);

                    // time always decreases towards zero
                    expect(oldUnbonding, errMsg).toBeGreaterThanOrEqual(newUnbonding);

                    const difference = oldUnbonding - newUnbonding;

                    // `waitEras(bondingDuration)` runs in parallel; when unbonding completes the
                    // next chunk disappears and millis snaps to zero (a full-era drop, not ~1 block).
                    if (newUnbonding === 0n) {
                        expect(oldUnbonding, errMsg).toBeGreaterThan(0n);
                        break;
                    }

                    expect(difference, errMsg).toBeGreaterThanOrEqual(0);
                    expect(difference, errMsg).toBeLessThanOrEqual(maxDecreaseMs);
                    if (difference <= blockTime * 2) {
                        expect(difference, errMsg).toBeGreaterThanOrEqual(blockTime);
                    }

                    oldUnbonding = newUnbonding;
                }
            }

            // configure proxy - used only for Full Unbond scenarios
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(api, nonProxiedCliFullUnbond, callerFullUnbond, proxy, wrongProxy);

            // wait for funds to become unlocked
            await erasCountdownPromise;
        }, 1_500_000);

        afterAll(() => {
            tearDownProxy(nonProxiedCliFullUnbond, proxy);
        });

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient funds" message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI('withdraw-unbonded');
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
                        CLI('withdraw-unbonded');
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

                try_catch_else_finally(
                    () => {
                        // try to withdraw again - should fail
                        CLI('withdraw-unbonded');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stderr).toContain(
                            'Cannot perform action, there are no unlocked funds to withdraw',
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
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

                try_catch_else_finally(
                    () => {
                        // try to withdraw again - should fail
                        nonProxiedCliPartialUnbond('withdraw-unbonded');
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stderr).toContain(
                            'Cannot perform action, there are no unlocked funds to withdraw',
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
            90_000,
        );
    });
});
