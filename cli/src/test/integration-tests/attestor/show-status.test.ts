import execa = require('execa');
import fs = require('fs');
import os = require('os');
import path = require('path');

import { execSync } from 'child_process';

import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { ALICE_NODE_URL, BOB_NODE_URL, initAliceKeyring, randomFundedAccount, waitEras, CLIBuilder } from '../helpers';
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../../blockchain-tests/pallets/supported-chains/consts';

describe('show-status', () => {
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
        try {
            CLI('attestor show-status');
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain("error: required option '--substrate-address [address]' not specified");
        }
    }, 30_000);

    // enable after fixing CSUB-1660
    it.skip('should error when required option --chain is not specified', () => {
        try {
            CLI(`attestor show-status --substrate-address ${attestor.address}`);
        } catch (error: any) {
            expect(error.exitCode).toEqual(1);
            expect(error.stderr).toContain("error: required option '-c, --chain [chain]' not specified");
        }
    }, 30_000);

    it('should display not an attestor when address is not an attestor', () => {
        // note: not registered yet!
        const result = CLI(`attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} is not an attestor`);
    }, 30_000);

    it('should display status Chill when attestor is registered but not active', async () => {
        // setup
        const caller = await randomFundedAccount(api, sudoSigner);
        CLI = CLIBuilder({ CC_SECRET: caller.secret });

        let result = CLI(`attestor register --chain ${chain_Anvil1_Key} --attestor ${attestor.address}`);
        expect(result.exitCode).toEqual(0);

        result = CLI(
            `attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key} --url ${BOB_NODE_URL}`,
        );
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Chill`);
    }, 60_000);

    // enable after fixing CSUB-1662
    test.skip('should display status Active when attestor is registered and active', async () => {
        // setup
        const caller = await randomFundedAccount(api, sudoSigner);
        CLI = CLIBuilder({ CC_SECRET: caller.secret });

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

        // test
        result = CLI(`attestor show-status --substrate-address ${attestor.address} --chain ${chain_Anvil1_Key}`);
        expect(result.exitCode).toEqual(0);
        expect(result.stdout).toContain(`Address ${attestor.address} status is Active`);
    }, 400_000);
});
