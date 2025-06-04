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

describe('register', () => {
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
        attestor = await randomFundedAccount(api, sudoSigner);

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

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
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
                CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
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
        'should register the attestor',
        () => {
            const result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
            // note: must call attestation.attest() and wait 1 era before it becomes active
        },
        100_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should fail when already registered',
        () => {
            // setup
            const result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            try {
                // call again
                CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain('attestation.AlreadyAttestor');
            }
        },
        90_000,
    );
});
