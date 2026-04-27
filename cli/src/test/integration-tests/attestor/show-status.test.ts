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
    let stash: any;
    let CLI: any;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        attestor = await randomFundedAccount(api, sudoSigner);
        stash = await randomFundedAccount(api, sudoSigner);

        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({});
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --evm-address is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor show-status');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '--evm-address [address]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should error when required option --chain is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor show-status --evm-address ${stash.ethEvmAddress}`);
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

    it('should display "no attestors" message when stash has none registered', () => {
        // note: nothing registered yet under this stash!
        const result = CLI(`attestor show-status --evm-address ${stash.ethEvmAddress} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(
            `No attestors registered for stash ${stash.ethEvmAddress} on chain ${chain_Anvil1_Key}`,
        );
    }, 30_000);

    it('should display status Idle for an attestor that was registered but never attested', async () => {
        // The EVM-derived stash address is what `register_attestor` records as the stash AccountId,
        // so it must be funded explicitly (separate from the sr25519 account funded by `randomFundedAccount`).
        await fundFromSudo(api, stash.evmStashAddress, parseAmount('1000'));
        const authenticatedCLI = CLIBuilder({ CC_SECRET: stash.secret });

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        result = CLI(
            `attestor show-status --evm-address ${stash.ethEvmAddress} --chain ${chain_Anvil1_Key} --url ${BOB_NODE_URL}`,
        );
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Idle`);
    }, 60_000);

    test('should display status Active for an attestor after election promotes it', async () => {
        await fundFromSudo(api, stash.evmStashAddress, parseAmount('1000'));
        const authenticatedCLI = CLIBuilder({ CC_SECRET: stash.secret });

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        // Transition attestor to Active via api.tx.attestation.attest() + election wait.
        // No external attestor binary needed.
        await activateAttestor(api, attestor, chain_Anvil1_Key);

        result = CLI(`attestor show-status --evm-address ${stash.ethEvmAddress} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Active`);
    }, 400_000);

    test('should list every attestor of the stash with its status', async () => {
        await fundFromSudo(api, stash.evmStashAddress, parseAmount('2000'));
        const authenticatedCLI = CLIBuilder({ CC_SECRET: stash.secret });

        const attestor2 = await randomFundedAccount(api, sudoSigner);

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor2.address}`);
        expect(result.exitCode).toEqual(0);

        result = CLI(`attestor show-status --evm-address ${stash.ethEvmAddress} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Idle`);
        expect(result.stdout).toContain(`Address ${attestor2.address} status is Idle`);
    }, 120_000);
});
