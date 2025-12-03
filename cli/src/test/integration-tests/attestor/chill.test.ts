// eslint-disable-next-line @typescript-eslint/no-require-imports
import execa = require('execa');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import fs = require('fs');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import path = require('path');

import { commandSync } from 'execa';
import { execSync } from 'child_process';

import { testIf, try_catch_else_finally } from '../../utils';
import {
    initAliceKeyring,
    randomFundedAccount,
    setUpProxy,
    tearDownProxy,
    waitEras,
    ALICE_NODE_URL,
    CLIBuilder,
} from '../helpers';
import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import {
    chain_Anvil1_Key,
    chain_Anvil1_Url,
    chain_Anvil2_Key,
} from '../../blockchain-tests/pallets/supported-chains/consts';

describe('chill', () => {
    let api: ApiPromise;
    let caller: any;
    let proxy: any;
    let wrongProxy: any;
    let sudoSigner: KeyringPair;
    let attestor: any;
    let CLI: any;
    let nonProxiedCli: any;
    let wrongCLI: any;

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
        wrongCLI = CLIBuilder({ CC_SECRET: wrongProxy.secret });
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
        try {
            commandSync('killall -9 attestor');
        } catch (_error: any) {
            // there may be no attestor running - don't throw an error
        }
        await api.disconnect();
    });

    it('should error when required option --chain is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor chill`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '-c, --chain [chain]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 90_000);

    it('should error when required option --attestor is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor chill --chain ${chain_Anvil1_Key}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("-a, --attestor [attestor]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 90_000);

    describe('when attestor is active', () => {
        beforeEach(async () => {
            // don't use execa/commandSync b/c they parse & quote the input and passing the mnemonic fails
            const secretSeed = execSync(
                `subkey inspect "${attestor.secret}" | grep 'Secret seed:' | cut -f2 -d: | tr -d ' '`,
            )
                .toString()
                .trim();
            expect(secretSeed.startsWith('0x')).toEqual(true);

            // warning: GitHub doesn't allow uploading files with colon in their name
            const logsDir = './logs';
            if (!fs.existsSync(logsDir)) {
                fs.mkdirSync(logsDir, { recursive: true });
            }

            const timeStamp = new Date().toISOString().replaceAll(':', '-');
            const logPrefix = path.join(logsDir, `attestor-${timeStamp}-log`);
            const args = [
                '--name',
                'ChillActive',
                '--secret',
                attestor.secret,
                '--cc3-url',
                ALICE_NODE_URL,
                '--eth-url',
                chain_Anvil1_Url,
                '--config',
                '../attestor_new/config.yaml',
            ];

            void execa('../target/release/attestor', args, {
                detached: true,
                stdout: fs.openSync(`${logPrefix}.stdout`, 'w'),
                stderr: fs.openSync(`${logPrefix}.stderr`, 'w'),
            });

            await waitEras(2, api);

            // make sure attestor was elected and is active
            const attestorsBefore: string[] = [];
            const entriesBefore = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
            for (const [_indx, account] of entriesBefore) {
                attestorsBefore.push(account.toString());
            }
            expect(attestorsBefore.length).toBeGreaterThan(0);
            expect(attestorsBefore).toContain(attestor.address);
        }, 360_000);

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient funds" message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI(`attestor chill --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stderr).toContain(
                            `Caller ${proxy.address} has insufficient funds to send the transaction`,
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
        );

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'not-a-proxy',
            'should error with proxy.NotProxy message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI(`attestor chill --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                    },
                    (error: any) => {
                        expect(error.exitCode).toEqual(1);
                        expect(error.stdout).toContain(
                            'Transaction failed with error: "proxy.NotProxy: Sender is not a proxy of the account to be proxied."',
                        );
                    },
                    () => {
                        throw new Error('cli was expected to fail but it did not');
                    },
                );
            },
        );

        testIf(
            process.env.PROXY_ENABLED === undefined ||
                process.env.PROXY_ENABLED === 'no' ||
                (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
            'should chill',
            async () => {
                // test
                const result = CLI(`attestor chill --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain('Transaction included at block');
                await waitEras(2, api);

                // make sure attestor is no longer active
                const attestorsAfter: string[] = [];
                const entriesAfter = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
                for (const [_indx, account] of entriesAfter) {
                    attestorsAfter.push(account.toString());
                }
                expect(attestorsAfter).not.toContain(attestor.address);
            },
            360_000,
        );
    });

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when attestor not registered for chain',
        () => {
            try_catch_else_finally(
                () => {
                    // note: we're registering to Anvil 1 above
                    CLI(`attestor chill --chain ${chain_Anvil2_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(
                        `There is not attestor ${attestor.address} for chain ${chain_Anvil2_Key}`,
                    );
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when caller is not an attestor stash',
        () => {
            try_catch_else_finally(
                () => {
                    // note: using a different caller to trigger a mismatch
                    wrongCLI(`attestor chill --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(
                        `Attestor ${attestor.address} is not owned by the keyring account ${wrongProxy.address}`,
                    );
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        90_000,
    );

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should error when attestor not active',
        () => {
            try_catch_else_finally(
                () => {
                    // note: not activated yet
                    CLI(`attestor chill --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(`Attestor ${attestor.address} is already chilled`);
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        90_000,
    );
});
