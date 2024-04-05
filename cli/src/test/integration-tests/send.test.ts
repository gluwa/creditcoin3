import { testIf } from '../utils';
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
    let proxy: any;
    let wrongProxy: any;
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
        wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
    }, 90_000);

    afterEach(() => {
        tearDownProxy(nonProxiedCli, proxy);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' &&
                process.env.PROXY_SECRET_VARIANT === 'valid-proxy' &&
                process.env.PROXY_TYPE === 'All'),
        'should be able to send CTC',
        () => {
            // Send money to Alice
            const result = CLI('send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');
        },
        60_000,
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try {
                CLI('send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    `Caller ${proxy.address} has insufficient funds to send the transaction`,
                );
            }
        },
        60_000,
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
        'should error with not-a-proxy message',
        () => {
            try {
                CLI('send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(`ERROR: ${wrongProxy.address} is not a proxy for ${caller.address}`);
            }
        },
        60_000,
    );

    testIf(
        process.env.PROXY_ENABLED === 'yes' &&
            process.env.PROXY_SECRET_VARIANT === 'valid-proxy' &&
            process.env.PROXY_TYPE !== 'All',
        'should error with no permission message',
        () => {
            try {
                CLI('send --amount 1 --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain(
                    `ERROR: The proxy ${proxy.address} for address ${caller.address} does not have permission to call extrinsics from the balances pallet`,
                );
            }
        },
        60_000,
    );
});
