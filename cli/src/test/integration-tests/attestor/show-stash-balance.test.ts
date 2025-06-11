import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { ALICE_NODE_URL, BOB_NODE_URL, initAliceKeyring, randomFundedAccount, CLIBuilder } from '../helpers';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('show-stash-balance', () => {
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

    it('should error when required option is not specified', () => {
        try {
            CLI('attestor show-stash-balance');
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
        }
    }, 30_000);

    it('should error when address is not an attestor', () => {
        try {
            // note: not registered yet and also not using caller.address, see below!
            CLI(`attestor show-stash-balance --substrate-address ${attestor.address}`);
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain(`No ledger found for ${attestor.address}`);
        }
    }, 30_000);

    it('should display balance when attestor is registered', async () => {
        // setup
        const caller = await randomFundedAccount(api, sudoSigner);
        CLI = CLIBuilder({ CC_SECRET: caller.secret });

        let result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        // note: using the caller address, not the attestor address
        result = CLI(`attestor show-stash-balance --substrate-address ${caller.address} --url ${BOB_NODE_URL}`);
        expect(result.exitCode).toEqual(0);

        expect(result.stdout).toContain(`Address: ${caller.address}`);
        expect(result.stdout).toContain('Transferable');
        expect(result.stdout).toContain('Locked');
        expect(result.stdout).toContain('Total');
        expect(result.stdout).toContain('TotalStake');
        expect(result.stdout).toContain('ActiveStake');
        expect(result.stdout).toContain('Unbonding');
        expect(result.stdout).toContain('CanWithdraw');
        expect(result.stdout).toContain('UnclaimedRewards');
    }, 60_000);
});
