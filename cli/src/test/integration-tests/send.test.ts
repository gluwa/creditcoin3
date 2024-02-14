import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

describe('Send command', () => {
    let api: ApiPromise;
    let caller: any;
    let proxyAccount: any;
    let sudoSigner: KeyringPair;
    let CLI: any;
    let nonProxiedCli: any;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        // Create a reference to sudo for funding accounts
        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        // Create and fund the test and proxy account
        caller = await randomFundedAccount(api, sudoSigner);
        proxyAccount = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });
        CLI = setUpProxy(nonProxiedCli, caller, proxyAccount);
    }, 60_000);

    afterAll(async () => {
        tearDownProxy(nonProxiedCli, proxyAccount);
        await api.disconnect();
    });

    it('should be able to send CTC', () => {
        // Send money to Alice
        const result = CLI(`send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain('Transaction included at block');
    }, 60_000);
});
