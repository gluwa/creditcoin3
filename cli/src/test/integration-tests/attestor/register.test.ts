import { testIf, try_catch_else_finally } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    fundFromSudo,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair, BN } from '../../../lib';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';
import { parseAmount } from '../../../commands/options';

describe('register', () => {
    let api: ApiPromise;
    let caller: any;
    let sudoSigner: KeyringPair;
    let attestor: any;
    let CLI: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        attestor = await randomFundedAccount(api, sudoSigner);

        // Create and fund the test account (sr25519)
        caller = await randomFundedAccount(api, sudoSigner);
        // Also fund the EVM-derived stash address (used by the precompile)
        await fundFromSudo(api, caller.evmStashAddress, parseAmount('1000'));

        CLI = CLIBuilder({ CC_SECRET: caller.secret });
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --attestor is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor register');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("-a, --attestor [attestor]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should error when required option --chain is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor register --attestor ${attestor.address}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '-c, --chain [chain]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    // NOTE: Proxy is not supported for EVM precompile calls. The stash must sign directly.
    testIf(false, 'proxy: should error with "Caller has insufficient funds" message', () => {
        // disabled: proxy not supported via EVM precompile
    });

    testIf(false, 'proxy: should error with proxy.NotProxy message', () => {
        // disabled: proxy not supported via EVM precompile
    });

    testIf(
        process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
        'should register the attestor',
        () => {
            const result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
            // note: must call attestation.attest() and wait 1 era before it becomes active
        },
        100_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
        'should fail when already registered',
        () => {
            // setup
            const result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            try_catch_else_finally(
                () => {
                    // call again
                    CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain('AlreadyAttestor');
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        90_000,
    );
});
