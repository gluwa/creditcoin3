import { commandSync } from 'execa';
import { describeIf } from '../utils';
import { CLI_PATH } from './helpers';
import { initEthKeyringPair, initKeyringPair } from '../../lib/account/keyring';

import { cryptoWaitReady } from '@polkadot/util-crypto';

let substrateSeedPhrase = '';
let substrateAccount = null;
let substrateAddress = '';
let expectedAssociatedEvmAddress = '';

let evmSeedPhrase = '';
let evmAccount = null;
let evmAddress = '';
let expectedAssociatedSubtrateAddress = '';

describeIf(
    process.env.PROXY_ENABLED === undefined || process.env.PROXY_ENABLED === 'no',
    'Convert Address command',
    () => {
        beforeAll(async () => {
            // Wait for crypto to be ready
            await cryptoWaitReady();

            substrateSeedPhrase = 'wheel practice idle spin artefact unlock coffee yellow mirror pudding fetch supreme';
            substrateAccount = initKeyringPair(substrateSeedPhrase);
            substrateAddress = substrateAccount.address;
            expectedAssociatedEvmAddress = '0x347557916f6abfc2b0862514b0e90708b422d992';

            // EVM address (uses index 0)
            evmSeedPhrase = 'drift glove bar million spare better spot afford pave horn annual bunker';
            evmAccount = initEthKeyringPair(evmSeedPhrase);
            evmAddress = evmAccount.address;
            expectedAssociatedSubtrateAddress = '5FQMKPxJuFBCeH7zQvwuKiRa3bMU41uYiumTkjXb1WeQxvf8';
        });

        it('should NOT convert an invalid EVM address', () => {
            const result = commandSync(`node ${CLI_PATH} convert-address --address 0x123`, { reject: false });

            expect(result.stderr).toContain('Not a valid Substrate or EVM address.');
        }, 60000);

        it('should NOT convert an invalid Substrate address', () => {
            const result = commandSync(
                `node ${CLI_PATH} convert-address --address 5FQMKPxJuFBCeH7zQvw0xa>>!jXb1WeQxvf8`,
                {
                    reject: false,
                },
            );

            expect(result.stderr).toContain('Not a valid Substrate or EVM address.');
        }, 60000);

        it('should convert a known Substrate address to a known EVM address', () => {
            const result = commandSync(`node ${CLI_PATH} convert-address --address ${substrateAddress}`);

            expect(result.stdout.toLowerCase()).toContain(expectedAssociatedEvmAddress.toLowerCase());
        }, 60000);

        it('should convert a known EVM address to a known Substrate address', () => {
            const result = commandSync(`node ${CLI_PATH} convert-address --address ${evmAddress}`);

            expect(result.stdout.toLowerCase()).toContain(expectedAssociatedSubtrateAddress.toLowerCase());
        }, 60000);
    },
);
