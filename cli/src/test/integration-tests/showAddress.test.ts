import { commandSync } from 'execa';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { CLI_PATH, randomTestAccount } from './helpers';


// TODO
//
// Make tests that cover the following cases:
// - show-address with no arguments (Substrate address from mnemonic)
// - show-address with --ecdsa (Substrate address from PK using ECDSA)
// - show-address with --eth (Ethereum address from mnemonic OR PK)
//
// NOTE: The --eth flag should only be available on show-address while the
//      --ecdsa flag should be available on all commands that require signing.

describe('Show address command', () => {
    it('should return the correct address', async () => {
        await cryptoWaitReady();

        const caller = randomTestAccount();

        const result = commandSync(`node ${CLI_PATH} show-address`, {
            env: {
                CC_SECRET: caller.secret,
            },
        });

        expect(result.stdout.split('Account address: ')[1]).toEqual(
            caller.address.toString()
        );
    }, 60000);
});