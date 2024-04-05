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
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getBalance } from '../../lib/balance';

describe('distribute-rewards', () => {
    let startingEra: number;
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

        startingEra = (await api.derive.session.info()).activeEra.toNumber();
        // make sure there is at least one era for which to distribute rewards
        await waitEras(2, api);
    }, 500_000);

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

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                // Alice is always a validator
                CLI(`distribute-rewards --era ${startingEra} --substrate-address ${sudoSigner.address}`);
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
                // Alice is always a validator
                CLI(`distribute-rewards --era ${startingEra} --substrate-address ${sudoSigner.address}`);
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
        'should distribute rewards',
        async () => {
            const oldBalance = await getBalance(sudoSigner.address, api);

            // Alice is always a validator
            const result = CLI(`distribute-rewards --era ${startingEra} --substrate-address ${sudoSigner.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await sleep(5000);
            const newBalance = await getBalance(sudoSigner.address, api);
            // https://polkadot.js.org/docs/api/start/types.basics/#working-with-numbers
            // .toNumber() can overflow: Number can only safely store up to 53 bits
            // .toBigInt() not available - Property 'toBigInt' does not exist on type 'BN'
            // try comparing the values directly instead of using .toBeGreaterThan()
            expect(newBalance.locked > oldBalance.locked).toBe(true);
            // WARNING: ^^^ by default reward destination is Staked!

            // try again - should error
            try {
                // Alice is always a validator
                CLI(`distribute-rewards --era ${startingEra} --substrate-address ${sudoSigner.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(
                    'staking.AlreadyClaimed: Rewards for this era have already been claimed for this validator',
                );
            }
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should fail when era not in history',
        () => {
            try {
                // Alice is always a validator
                CLI(`distribute-rewards --era 999999 --substrate-address ${sudoSigner.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    'Failed to distribute rewards: Era 999999 is not included in history; only the past 84 eras are eligible',
                );
            }
        },
        60_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should fail when address is not a validator',
        () => {
            try {
                // `caller` is NOT a validator
                CLI(`distribute-rewards --era ${startingEra} --substrate-address ${caller.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain('staking.NotStash: Not a stash account');
            }
        },
        60_000,
    );
});
