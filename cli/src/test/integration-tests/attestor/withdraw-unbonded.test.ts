import { testIf, try_catch_else_finally, forElapsedBlocks } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    waitEras,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('withdraw-unbonded', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let sudoSigner: KeyringPair;
    let attestor: any;
    let CLI: any;
    let nonProxiedCli: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();

        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

        proxy = await randomFundedAccount(api, sudoSigner);
        const wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(api, nonProxiedCli, caller, proxy, wrongProxy);

        attestor = await randomFundedAccount(api, sudoSigner);

        // NOTE: caller/proxy is the STASH for a random attestor on the Anvil1 chain
        // use CLI b/c it differentiates b/w caller/proxy accounts while direct API calls don't
        let result = nonProxiedCli(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        await forElapsedBlocks(api, { minBlocks: 1 });

        // after unregistering the unbonding period starts
        result = nonProxiedCli(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
    }, 150_000);

    afterAll(async () => {
        tearDownProxy(nonProxiedCli, proxy);

        await api.disconnect();
    }, 120_000);

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should exit when caller is not a stash',
        async () => {
            const newCaller = await randomFundedAccount(api, sudoSigner);
            const nonStashCLI = CLIBuilder({ CC_SECRET: newCaller.secret });

            const result = nonStashCLI(`attestor withdraw-unbonded`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain(`No unbonded funds to withdraw for address ${newCaller.address}`);
        },
        30_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should exit when funds have not been unlocked yet',
        () => {
            // note: not waiting for unbonding period to finish
            const result = CLI(`attestor withdraw-unbonded`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain(`No unbonded funds to withdraw`);
        },
        30_000,
    );

    describe('when funds have been unlocked', () => {
        beforeAll(async () => {
            // wait for funds to be unlocked!
            const unbondingPeriod: number = api.consts.attestation.bondingDuration.toNumber();
            await waitEras(unbondingPeriod, api); // ~5 minutes
        }, 400_000);

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient funds" message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI(`attestor withdraw-unbonded`);
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
                        CLI(`attestor withdraw-unbonded`);
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
            'should succeed when funds have been unlocked',
            () => {
                const result = CLI(`attestor withdraw-unbonded`);
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Unbonded funds available to withdraw:');
                expect(result.stdout).toContain('Transaction included at block');
            },
        );
    });
});
