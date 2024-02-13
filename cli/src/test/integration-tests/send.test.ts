import { commandSync } from 'execa';
import { initAliceKeyring, randomFundedAccount, ALICE_NODE_URL, CLI_PATH } from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

describe('Send command', () => {
    let api: ApiPromise;
    let caller: any;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should be able to send CTC', () => {
        const result = commandSync(
            `node ${CLI_PATH} send --substrate-address 5HDRB6edmWwwh6aCDKrRSbisV8iFHdP7jDy18U2mt9w2wEkq --amount 10`,
            {
                env: {
                    CC_SECRET: caller.secret,
                },
            },
        );

        expect(result.stdout).toContain('Transaction included');
    }, 60_000);
});
