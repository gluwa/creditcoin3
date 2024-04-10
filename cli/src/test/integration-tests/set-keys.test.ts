import { testIf, sleep } from '../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    ALICE_NODE_URL,
    CLIBuilder,
} from './helpers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';

describe('set-keys', () => {
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

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        // Create and fund the test account
        caller = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });
    }, 60_000);

    describe('when NOT bonded', () => {
        beforeEach(() => {
            CLI = nonProxiedCli;
        });

        it('should error when NO key options are specified', () => {
            try {
                CLI('set-keys');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stdout).toContain('Must specify keys to set or generate new ones using the --rotate flag');
            }
        });

        it('should error when BOTH key options are specified', () => {
            try {
                CLI('set-keys --rotate --keys "test-me"');
            } catch (error: any) {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain(
                    'Must either specify keys or rotate to generate new ones, can not do both',
                );
            }
        });
    });

    describe('when ALREADY bonded', () => {
        beforeEach(async () => {
            // bond before calling set-keys
            const result = nonProxiedCli(`bond --amount 12`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // create and fund the proxy account
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
        }, 90_000);

        afterEach(() => {
            tearDownProxy(nonProxiedCli, proxy);
        });

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with account balance too low message',
            () => {
                try {
                    CLI('set-keys --rotate');
                } catch (error: any) {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stderr).toContain(
                        'Invalid Transaction: Inability to pay some fees , e.g. account balance too low',
                    );
                }
            },
        );

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
            'should error with proxy.NotProxy message',
            () => {
                try {
                    CLI('set-keys --rotate');
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
            'should set new keys',
            async () => {
                const oldSessionKeys = await api.query.session.nextKeys(caller.address);
                const result = CLI('set-keys --rotate');
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');

                // wait 2 seconds for nodes to sync
                await sleep(2000);
                const newSessionKeys = await api.query.session.nextKeys(caller.address);
                expect(newSessionKeys.toHex()).not.toBe(oldSessionKeys.toHex());
            },
            60_000,
        );
    });
});
