import { ALICE_NODE_URL, BOB_NODE_URL, initAliceKeyring, randomFundedAccount, CLIBuilder } from './helpers';
import { newApi, ApiPromise, BN, KeyringPair, MICROUNITS_PER_CTC } from '../../lib';

describe('balance', () => {
    let api: ApiPromise;
    let randomAccount: any;
    let CLI: any;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        randomAccount = await randomFundedAccount(api, sudoSigner);

        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({});
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option is not specified', () => {
        try {
            CLI('balance');
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
        }
    }, 30_000);

    it('should display balance', () => {
        const result = CLI(`balance --substrate-address ${randomAccount.address} --url ${ALICE_NODE_URL}`);

        expect(result.exitCode).toEqual(0);

        expect(result.stdout).toContain(`Address: ${randomAccount.address}`);
        expect(result.stdout).toContain('Transferable');
        expect(result.stdout).toContain('Locked');
        expect(result.stdout).toContain('Bonded');
        expect(result.stdout).toContain('EVM');
        expect(result.stdout).toContain('Unbonding');
        expect(result.stdout).toContain('Total');
    }, 30_000);

    it('result should be JSON when --json specified', () => {
        // setup - transfer some funds to the associated EVM account
        const fiftyCtc = MICROUNITS_PER_CTC.mul(new BN(50));
        const cli = CLIBuilder({ CC_SECRET: randomAccount.secret });
        let result = cli(`evm fund --evm-address ${randomAccount.evmAddress} --amount 50 --url ${BOB_NODE_URL}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included');

        // test - grab the balance - non-authenticated
        result = CLI(`balance --json --substrate-address ${randomAccount.address} --url ${BOB_NODE_URL}`);
        expect(result.exitCode).toEqual(0);

        const parsed = JSON.parse(result.stdout);

        expect(parsed).toHaveProperty('balance');
        expect(parsed).toHaveProperty('balance.address', randomAccount.address);
        expect(parsed).toHaveProperty('balance.transferable');
        expect(parsed).toHaveProperty('balance.bonded', '0');
        expect(parsed).toHaveProperty('balance.evm', fiftyCtc.toString());
        expect(parsed).toHaveProperty('balance.locked', '0');
        expect(parsed).toHaveProperty('balance.unbonding', '0');

        const expected = new BN(parsed.balance.transferable).add(new BN(fiftyCtc.toString()));
        expect(parsed).toHaveProperty('balance.total', expected.toString());
    }, 60_000);
});
