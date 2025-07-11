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
import { Option, U128 } from '@polkadot/types-codec';

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
import { newApi, ApiPromise, BN, KeyringPair } from '../../../lib';
import { toCTCString } from '../../../lib/balance';
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('claim-rewards', () => {
    let api: ApiPromise;
    let stash: any;
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
        stash = await randomFundedAccount(api, sudoSigner);
        nonProxiedCli = CLIBuilder({ CC_SECRET: stash.secret });

        proxy = await randomFundedAccount(api, sudoSigner);
        wrongProxy = await randomFundedAccount(api, sudoSigner);
        CLI = await setUpProxy(nonProxiedCli, stash, proxy, wrongProxy);
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

    describe('when rewards are present', () => {
        let attestor: any;

        beforeEach(async () => {
            attestor = await randomFundedAccount(api, sudoSigner);

            const nonce = await api.rpc.system.accountNextIndex(sudoSigner.address);
            await api.tx.sudo
                .sudo(api.tx.attestation.setTargetSampleSize(chain_Anvil1_Key, 1))
                .signAndSend(sudoSigner, { nonce });

            const result = nonProxiedCli(
                `attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`,
            );
            expect(result.exitCode).toEqual(0);

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
            const attestorsBefore: string[] = [];
            const entriesBefore = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
            for (const [_indx, account] of entriesBefore) {
                attestorsBefore.push(account.toString());
            }
            expect(attestorsBefore.length).toBeGreaterThan(0);
            expect(attestorsBefore).toContain(attestor.address);

            // wait again for rewards to accumulate
            await waitEras(2, api);

            // stash has accumulated some rewards already
            const accumulatedRewards = (await api.query.attestation.accumulatedRewards(stash.address)) as Option<U128>;
            expect(accumulatedRewards.isSome).toBeTruthy();
        }, 700_000);

        testIf(
            process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'no-funds',
            'should error with "Caller has insufficient funds" message',
            () => {
                try_catch_else_finally(
                    () => {
                        CLI(`attestor claim-rewards`);
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
                        CLI(`attestor claim-rewards`);
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
            'should claim rewards',
            async () => {
                const accumulatedRewards = (await api.query.attestation.accumulatedRewards(
                    stash.address,
                )) as Option<U128>;
                const expectedReward = BigInt(accumulatedRewards.unwrap().toString());
                const displayedReward = toCTCString(new BN(expectedReward.toString()), 4);

                // test
                const result = CLI(`attestor claim-rewards`);
                expect(result.exitCode).toEqual(0);
                expect(result.stdout).toContain(
                    `Rewards available to claim: ${displayedReward} for address ${stash.address}`,
                );
                expect(result.stdout).toContain('Transaction included at block');
            },
            30_000,
        );
    });

    testIf(
        process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
        'should not claim rewards when none are present',
        () => {
            // test
            const result = CLI(`attestor claim-rewards`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('No rewards to claim for address');
        },
        30_000,
    );
});
