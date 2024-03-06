import { ALICE_NODE_URL, BOB_NODE_URL, randomTestAccount, CLIBuilder } from './helpers';
import { newApi, ApiPromise } from '../../lib';

describe('balance', () => {
    let api: ApiPromise;
    let randomAccount: any;
    let CLI: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));
    });

    beforeEach(() => {
        randomAccount = randomTestAccount();

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
        const result = CLI(`balance --json --substrate-address ${randomAccount.address} --url ${BOB_NODE_URL}`);
        expect(result.exitCode).toEqual(0);

        const parsed = JSON.parse(result.stdout);

        expect(parsed).toHaveProperty('balance');
        expect(parsed).toHaveProperty('balance.address', randomAccount.address);
        expect(parsed).toHaveProperty('balance.transferable', '0');
        expect(parsed).toHaveProperty('balance.bonded', '0');
        expect(parsed).toHaveProperty('balance.evm', '0');
        expect(parsed).toHaveProperty('balance.locked', '0');
        expect(parsed).toHaveProperty('balance.unbonding', '0');
        expect(parsed).toHaveProperty('balance.total', '0');

        // todo: make sure Total reflects EVM, see CSUB-1061
        // https://github.com/gluwa/creditcoin3/pull/245
    }, 30_000);
});
