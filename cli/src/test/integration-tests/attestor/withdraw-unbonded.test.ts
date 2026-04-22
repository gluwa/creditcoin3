import { testIf, forElapsedBlocks } from '../../utils';
import { initAliceKeyring, randomFundedAccount, fundFromSudo, waitEras, ALICE_NODE_URL, CLIBuilder } from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';
import { parseAmount } from '../../../commands/options';

describe('withdraw-unbonded', () => {
    let api: ApiPromise;
    let caller: any;
    let sudoSigner: KeyringPair;
    let attestor: any;
    let CLI: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();

        // Create and fund the test account (sr25519 + EVM stash)
        caller = await randomFundedAccount(api, sudoSigner);
        await fundFromSudo(api, caller.evmStashAddress, parseAmount('1000'));
        CLI = CLIBuilder({ CC_SECRET: caller.secret });

        attestor = await randomFundedAccount(api, sudoSigner);

        // NOTE: caller is the STASH for a random attestor on the Anvil1 chain
        let result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        await forElapsedBlocks(api, { minBlocks: 1 });

        // after unregistering the unbonding period starts
        result = CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
    }, 150_000);

    afterAll(async () => {
        await api.disconnect();
    }, 120_000);

    testIf(
        process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
        'should exit when caller is not a stash',
        async () => {
            const newCaller = await randomFundedAccount(api, sudoSigner);
            const nonStashCLI = CLIBuilder({ CC_SECRET: newCaller.secret });

            const result = nonStashCLI(`attestor withdraw-unbonded`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain(`No unbonded funds to withdraw for address ${newCaller.evmStashAddress}`);
        },
        30_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
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

        // NOTE: Proxy is not supported for EVM precompile calls.
        testIf(false, 'proxy: should error with "Caller has insufficient funds" message', () => {
            // disabled: proxy not supported via EVM precompile
        });

        testIf(false, 'proxy: should error with proxy.NotProxy message', () => {
            // disabled: proxy not supported via EVM precompile
        });

        testIf(
            process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
            'should succeed when funds have been unlocked',
            () => {
                const result = CLI(`attestor withdraw-unbonded`);
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Unbonded funds available to withdraw');
                expect(result.stdout).toContain('Transaction included at block');
            },
        );
    });
});
