import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

const testDescription = () => {
    let description = 'should be able to send CTC';

    if (process.env.PROXY_SECRET_VARIANT !== 'valid-proxy') {
        description = 'should error with not-a-proxy message';
    } else if (process.env.PROXY_TYPE === 'Staking' || process.env.PROXY_TYPE === 'NonTransfer') {
        description = `should error with no permission message for proxy type ${process.env.PROXY_TYPE}`;
    }

    return description;
};

describe('Send command', () => {
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
        proxy = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });
        CLI = await setUpProxy(nonProxiedCli, caller, proxy);
    }, 60_000);

    afterEach(() => {
        tearDownProxy(nonProxiedCli, proxy);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it(
        testDescription(),
        () => {
            // Send money to Alice
            try {
                const result = CLI(`send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY`);
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);

                if (process.env.PROXY_SECRET_VARIANT !== 'valid-proxy') {
                    expect(error.stdout).toContain(`ERROR: ${proxy.address} is not a proxy for ${caller.address}`);
                } else if (process.env.PROXY_TYPE === 'Staking' || process.env.PROXY_TYPE === 'NonTransfer') {
                    expect(error.stdout).toContain(
                        `ERROR: The proxy ${proxy.address} for address ${caller.address} does not have permission to call extrinsics from the balances pallet`,
                    );
                }
            }
        },
        60_000,
    );
});
