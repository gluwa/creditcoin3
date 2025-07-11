// eslint-disable-next-line @typescript-eslint/no-require-imports
import execa = require('execa');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import fs = require('fs');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import os = require('os');

// eslint-disable-next-line @typescript-eslint/no-require-imports
import path = require('path');

import { execSync } from 'child_process';

import { newApi, ApiPromise, BN, KeyringPair } from '../../../lib';
import { toCTCString } from '../../../lib/balance';
import { try_catch_else_finally } from '../../utils';
import { ALICE_NODE_URL, BOB_NODE_URL, initAliceKeyring, randomFundedAccount, waitEras, CLIBuilder } from '../helpers';
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('show-unclaimed-rewards', () => {
    let api: ApiPromise;
    let attestor: any;
    let CLI: any;
    let sudoSigner: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));

        sudoSigner = initAliceKeyring();
    });

    beforeEach(async () => {
        attestor = await randomFundedAccount(api, sudoSigner);

        // eslint-disable-next-line @typescript-eslint/naming-convention
        CLI = CLIBuilder({});
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('should error when required option --substrate-address is not specified', () => {
        try_catch_else_finally(
            () => {
                CLI('attestor show-unclaimed-rewards');
            },
            (error: any) => {
                expect(error.exitCode).toEqual(1);
                expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
            },
            () => {
                throw new Error('cli was expected to fail but it did not');
            },
        );
    }, 30_000);

    it('should display no rewards when address is not an attestor', () => {
        // note: not registered yet, also using attestor's address instead of stash address!
        const result = CLI(
            `attestor show-unclaimed-rewards --substrate-address ${attestor.address} --url ${BOB_NODE_URL}`,
        );
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`No rewards to claim for address ${attestor.address}`);
    }, 30_000);

    it('should display rewards when attestor is registered and active', async () => {
        // setup
        const nonce = await api.rpc.system.accountNextIndex(sudoSigner.address);
        await api.tx.sudo
            .sudo(api.tx.attestation.setTargetSampleSize(chain_Anvil1_Key, 1))
            .signAndSend(sudoSigner, { nonce });

        const stash = await randomFundedAccount(api, sudoSigner);
        CLI = CLIBuilder({ CC_SECRET: stash.secret });

        let result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
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
        const activeAttestorsForAnvil1: string[] = [];
        const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
        for (const [_indx, account] of entriesForAnvil1) {
            activeAttestorsForAnvil1.push(account.toString());
        }
        expect(activeAttestorsForAnvil1.length).toBeGreaterThan(0);
        expect(activeAttestorsForAnvil1).toContain(attestor.address);

        // wait again for rewards to accumulate
        await waitEras(2, api);

        // stash has accumulated some rewards already
        const accumulatedRewards = await api.query.attestation.accumulatedRewards(stash.address);
        expect(accumulatedRewards.isSome).toBeTruthy();
        const expectedReward = BigInt(accumulatedRewards.unwrap().toString());
        const displayedReward = toCTCString(new BN(expectedReward.toString()), 4);
        expect(expectedReward).toBeGreaterThan(0);

        // test
        result = CLI(`attestor show-unclaimed-rewards --substrate-address ${stash.address}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Unclaimed rewards for : ${stash.address} is ${displayedReward}`);
    }, 700_000);
});
