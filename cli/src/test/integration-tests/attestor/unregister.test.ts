// eslint-disable-next-line @typescript-eslint/no-require-imports
import execa = require('execa');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import fs = require('fs');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import os = require('os');

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
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../../blockchain-tests/pallets/supported-chains/consts';

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
        try {
            commandSync('killall -9 attestor');
        } catch (_error: any) {
            // there may be no attestor running - don't throw an error
        }
        await api.disconnect();
    });

    it('should error when required option --attestor is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor unregister');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("-a, --attestor [attestor]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should error when required option --chain is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI(`attestor unregister --attestor ${attestor.address}`);
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '-c, --chain [chain]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    testIf(
        process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
        'should error with "Caller has insufficient funds" message',
        () => {
            try_catch_else_finally(
                () => {
                    CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
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
                    CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
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

            try_catch_else_finally(
                () => {
                    // call again
                    CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(`Address ${attestor.address} is not an attestor`);
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
        'should fail when still active',
        async () => {
            // don't use execa/commandSync b/c they parse & quote the input and passing the mnemonic fails
            const secretSeed = execSync(
                `subkey inspect "${attestor.secret}" | grep 'Secret seed:' | cut -f2 -d: | tr -d ' '`,
            )
                .toString()
                .trim();
            expect(secretSeed.startsWith('0x')).toEqual(true);

            // warning: GitHub doesn't allow uploading files with colon in their name
            const timeStamp = new Date().toISOString().replaceAll(':', '-');
            const logPrefix = path.join(os.tmpdir(), `attestor-${timeStamp}-log`);
            void execa(
                '../target/release/attestor',
                `--verbose --cc3-key ${secretSeed} --cc3-rpc-url ${ALICE_NODE_URL} --eth-rpc-url ${chain_Anvil1_Url}`.split(
                    ' ',
                ),
                {
                    detached: true,
                    stdout: fs.openSync(`${logPrefix}.stdout`, 'w'),
                    stderr: fs.openSync(`${logPrefix}.stderr`, 'w'),
                },
            );
            await waitEras(2, api);

            // make sure attestor was elected and is active
            const activeAttestorsForAnvil1: string[] = [];
            const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
            for (const [_indx, account] of entriesForAnvil1) {
                activeAttestorsForAnvil1.push(account.toString());
            }
            expect(activeAttestorsForAnvil1.length).toBeGreaterThan(0);
            expect(activeAttestorsForAnvil1).toContain(attestor.address);

            // test
            try_catch_else_finally(
                () => {
                    CLI(`attestor unregister --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
                },
                (error: any) => {
                    expect(error.exitCode).toEqual(1);
                    expect(error.stdout).toContain(
                        `Address ${attestor.address} status is Active. Please chill the attestor first`,
                    );
                },
                () => {
                    throw new Error('cli was expected to fail but it did not');
                },
            );
        },
        360_000,
    );
});
