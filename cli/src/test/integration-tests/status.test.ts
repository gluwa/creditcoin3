import { ALICE_NODE_URL, BOB_NODE_URL, randomTestAccount, CLIBuilder } from './helpers';
import { newApi, ApiPromise } from '../../lib';

describe('status', () => {
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
            CLI('status --chain');
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
        }
    }, 30_000);

    it('should display validator & chain status when both are requested', () => {
        const result = CLI(`status --chain --substrate-address ${randomAccount.address} --url ${ALICE_NODE_URL}`);

        expect(result.exitCode).toEqual(0);

        expect(result.stdout).toContain('Chain status:');
        expect(result.stdout).toContain('Best Block');
        expect(result.stdout).toContain('Best Finalized Block');
        expect(result.stdout).toContain('Era');
        expect(result.stdout).toContain('Session');

        expect(result.stdout).toContain(`Validator ${randomAccount.address}:`);
        expect(result.stdout).toContain('Bonded');
        expect(result.stdout).toContain('Validating');
        expect(result.stdout).toContain('Waiting');
        expect(result.stdout).toContain('Active');
        expect(result.stdout).toContain('Can withdraw');
        expect(result.stdout).toContain('Next unlocking');
    }, 30_000);

    it('should display only validator status when --chain is not specified', () => {
        const result = CLI(`status --substrate-address ${randomAccount.address} --url ${BOB_NODE_URL}`);

        expect(result.exitCode).toEqual(0);

        expect(result.stdout).not.toContain('Chain status:');
        expect(result.stdout).not.toContain('Best Block');
        expect(result.stdout).not.toContain('Best Finalized Block');
        expect(result.stdout).not.toContain('Era');
        expect(result.stdout).not.toContain('Session');

        expect(result.stdout).toContain(`Validator ${randomAccount.address}:`);
        expect(result.stdout).toContain('Bonded');
        expect(result.stdout).toContain('Validating');
        expect(result.stdout).toContain('Waiting');
        expect(result.stdout).toContain('Active');
        expect(result.stdout).toContain('Can withdraw');
        expect(result.stdout).toContain('Next unlocking');
    }, 30_000);
});
