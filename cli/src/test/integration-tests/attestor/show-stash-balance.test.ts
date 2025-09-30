import { newApi } from '../../../lib';
import { try_catch_else_finally } from '../../utils';
import { ALICE_NODE_URL, initAliceKeyring, randomFundedAccount, CLIBuilder } from '../helpers';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('show-stash-balance', () => {
    let CLI: any;

    beforeEach(() => {
        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({});
    });

    it('should error when required option is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor show-stash-balance');
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

    it('should error when address is not an attestor', () => {
        const ferdie = '5CiPPseXPECbkjWCa6MnjNokrgYjMqmKndv2rSnekmSK2DjL';

        try_catch_else_finally(
            () => {
                // note: not registered yet and also not using caller.address, see below!
                CLI(`attestor show-stash-balance --substrate-address ${ferdie}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(`No ledger found for ${ferdie}`);
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should display balance when attestor is registered', async () => {
        // setup - see commit log for the reasoning why this isn't in beforeAll()
        const sudoSigner = initAliceKeyring();
        const { api } = await newApi(ALICE_NODE_URL);

        const caller = await randomFundedAccount(api, sudoSigner);
        const attestor = await randomFundedAccount(api, sudoSigner);

        const authenticatedCLI = CLIBuilder({ CC_SECRET: caller.secret });

        let result = authenticatedCLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        // note: using the caller address, not the attestor address
        result = CLI(`attestor show-stash-balance --substrate-address ${caller.address}`);
        expect(result.exitCode).toEqual(0);

        expect(result.stdout).toContain(`Address: ${caller.address}`);
        expect(result.stdout).toContain('Transferable');
        expect(result.stdout).toContain('Locked');
        expect(result.stdout).toContain('Total');
        expect(result.stdout).toContain('TotalStake');
        expect(result.stdout).toContain('ActiveStake');
        expect(result.stdout).toContain('Unbonding');
        expect(result.stdout).toContain('CanWithdraw');

        // teardown
        await api.disconnect();
    }, 90_000);
});
