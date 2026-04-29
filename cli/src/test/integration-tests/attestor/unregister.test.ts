import { try_catch_else_finally } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    fundFromSudo,
    activateAttestor,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';
import { parseAmount } from '../../../commands/options';

// NOTE: The attestor-stash precompile uses the EVM `msg.sender` as the origin
// of the dispatched pallet call. Proxy-signed attestor operations are not
// supported on the EVM path; the stash must sign directly. No proxy matrix.
describe('unregister', () => {
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
        // Create and fund the test account (sr25519 + EVM stash)
        caller = await randomFundedAccount(api, sudoSigner);
        await fundFromSudo(api, caller.evmStashAddress, parseAmount('1000'));
        CLI = CLIBuilder({ CC_SECRET: caller.secret });

        attestor = await randomFundedAccount(api, sudoSigner);

        // NOTE: caller is the STASH for a random attestor on the Anvil1 chain
        const result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
    }, 150_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --attestor is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor unregister');
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
                CLI(`attestor unregister --attestor ${attestor.address}`);
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

    test('should unregister the attestor', () => {
        const result = CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included at block');
    }, 100_000);

    test('should fail when already unregistered', () => {
        // setup
        const result = CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included at block');

        try_catch_else_finally(
            () => {
                // call again
                CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`Address ${attestor.address} is not an attestor`);
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 90_000);

    test('should fail when still active', async () => {
        await activateAttestor(api, attestor, chain_Anvil1_Key);

        try_catch_else_finally(
            () => {
                CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Address ${attestor.address} status is Active. Please chill the attestor first`,
                );
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 360_000);
});
