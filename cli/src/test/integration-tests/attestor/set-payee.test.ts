import { try_catch_else_finally } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';

describe('setPayee', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
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
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

        proxy = await randomFundedAccount(api, sudoSigner);
        const wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
    }, 120_000);

    afterEach(() => {
        tearDownProxy(nonProxiedCli, proxy);
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --payee is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor set-payee');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '-p, --payee [payee]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);
});
