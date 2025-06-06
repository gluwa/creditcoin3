import { testIf } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { chain_Anvil1_Key } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('unregister', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let sudoSigner: KeyringPair;
    let attestor: any;
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

        attestor = await randomFundedAccount(api, sudoSigner);

        // NOTE: caller/proxy is the STASH for a random attestor on the Anvil1 chain
        // use CLI b/c it differentiates b/w caller/proxy accounts while direct API calls don't
        const result = nonProxiedCli(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);
    }, 150_000);

    afterEach(() => {
        tearDownProxy(nonProxiedCli, proxy);
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Caller ${proxy.address} has insufficient funds to send the transaction`,
                );
            }
        },
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
        'should error with proxy.NotProxy message',
        () => {
            try {
                CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(
                    'Transaction failed with error: "proxy.NotProxy: Sender is not a proxy of the account to be proxied."',
                );
            }
        },
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should unregister the attestor',
        () => {
            const result = CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        },
        100_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should fail when already unregistered',
        () => {
            // setup
            const result = CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            try {
                // call again
                CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(`Address ${attestor.address} is not an attestor`);
            }
        },
        90_000,
    );
});
