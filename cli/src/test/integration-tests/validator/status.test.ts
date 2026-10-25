import { ALICE_NODE_URL, randomTestAccount, CLIBuilder } from '../helpers';
import { newApi, ApiPromise } from '../../../lib';

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

    it('should display validator & chain status when both are requested', () => {
        const result = CLI(`status --substrate-address ${randomAccount.address} --url ${ALICE_NODE_URL}`);

        expect(result.exitCode).toEqual(0);

        expect(result.stdout).toContain('Active:');
        expect(result.stdout).toContain('Current:');
        expect(result.stdout).toContain('Session:');
        expect(result.stdout).toContain('Block:');
        expect(result.stdout).toContain('Finalized:');

        expect(result.stdout).toContain(`Validator ${randomAccount.address}:`);
        expect(result.stdout).toContain('Bonded');
        expect(result.stdout).toContain('Validating');
        expect(result.stdout).toContain('Waiting');
        expect(result.stdout).toContain('Active');
        expect(result.stdout).toContain('Can withdraw');
        expect(result.stdout).toContain('Next unlocking');
    }, 30_000);
});
