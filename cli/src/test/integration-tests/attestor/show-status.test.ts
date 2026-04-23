import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { try_catch_else_finally } from '../../utils';
import {
    ALICE_NODE_URL,
    BOB_NODE_URL,
    initAliceKeyring,
    randomFundedAccount,
    fundFromSudo,
    activateAttestor,
    CLIBuilder,
} from '../helpers';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';
import { parseAmount } from '../../../commands/options';

describe('show-status', () => {
    let api: ApiPromise;
    let attestor: any;
    let CLI: any;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        attestor = await randomFundedAccount(api, sudoSigner);

        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({});
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --substrate-address is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor show-status');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should error when required option --chain is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor show-status --substrate-address ${attestor.address}`);
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

    it('should display not an attestor when address is not an attestor', () => {
        // note: not registered yet!
        const result = CLI(`attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} is not an attestor`);
    }, 30_000);

    it('should display status Chill when attestor is registered but not active', async () => {
        // setup
        const caller = await randomFundedAccount(api, sudoSigner);
        // Fund the EVM-derived stash for precompile calls
        await fundFromSudo(api, caller.evmStashAddress, parseAmount('1000'));
        const authenticatedCLI = CLIBuilder({ CC_SECRET: caller.secret });

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        result = CLI(
            `attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key} --url ${BOB_NODE_URL}`,
        );
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Chill`);
    }, 60_000);

    test('should display status Active when attestor is registered and active', async () => {
        const caller = await randomFundedAccount(api, sudoSigner);
        await fundFromSudo(api, caller.evmStashAddress, parseAmount('1000'));
        const authenticatedCLI = CLIBuilder({ CC_SECRET: caller.secret });

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        // Transition attestor to Active via api.tx.attestation.attest() + election wait.
        // No external attestor binary needed.
        await activateAttestor(api, attestor, chain_Anvil1_Key);

        result = CLI(`attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Active`);
    }, 400_000);
});
