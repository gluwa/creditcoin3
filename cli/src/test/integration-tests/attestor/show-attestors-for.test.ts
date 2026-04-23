import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { try_catch_else_finally } from '../../utils';
import {
    ALICE_NODE_URL,
    initAliceKeyring,
    randomFundedAccount,
    fundFromSudo,
    activateAttestor,
    CLIBuilder,
} from '../helpers';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';
import { parseAmount } from '../../../commands/options';

describe('show-attestors-for', () => {
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

    it('should error when required option --substrate-address is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor show-attestors-for');
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
                CLI(`attestor show-attestors-for --substrate-address ${stash.address}`);
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

    it('should display empty output when stash does not have attestors', () => {
        // note: attestor not registered yet!
        const result = CLI(
            `attestor show-attestors-for --substrate-address ${stash.address} --chain ${chain_Anvil1_Key}`,
        );
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toEqual('');
    }, 30_000);

    describe('when attestor is registered and active', () => {
        beforeEach(async () => {
            // The attestor-stash precompile uses the EVM-derived stash (HashedAddressMapping),
            // not the sr25519 address funded by `randomFundedAccount`. Fund it explicitly so
            // `register_attestor` doesn't fail with `InsufficientBalance`.
            await fundFromSudo(api, stash.evmStashAddress, parseAmount('1000'));
            const authenticatedCLI = CLIBuilder({ CC_SECRET: stash.secret });

            const result = authenticatedCLI(
                `attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`,
            );
            expect(result.exitCode).toEqual(0);

            // Transition the registered attestor to Active by submitting attest()
            // directly from the test; no need to spawn the external attestor binary.
            await activateAttestor(api, attestor, chain_Anvil1_Key);
        }, 360_000);

        it('should display empty output when passing attestor address as argument', () => {
            // note: using attestor's address instead of stash address!
            const result = CLI(
                `attestor show-attestors-for --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key}`,
            );
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toEqual('');
        }, 30_000);

        it('should display attestor address when passing stash address as argument', () => {
            // The attestor-stash precompile records the stash as the EVM-derived
            // AccountId (HashedAddressMapping from the caller's EVM address), not
            // the sr25519 address. `show-attestors-for` matches on the recorded
            // `stash` field, so query by `stash.evmStashAddress`.
            const result = CLI(
                `attestor show-attestors-for --substrate-address ${stash.evmStashAddress} --chain ${chain_Anvil1_Key}`,
            );
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain(`Address ${attestor.address} is an attestor for chain ${chain_Anvil1_Key}`);
        }, 30_000);
    });
});
