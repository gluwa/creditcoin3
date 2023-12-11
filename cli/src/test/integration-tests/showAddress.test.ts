import { commandSync } from 'execa';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { CLI_PATH, randomTestAccount } from './helpers';
import { parseAddressInternal, parseEVMAddressInternal } from '../../lib/parsing';
import { substrateAddressToEvmAddress } from '../../lib/evm/address';

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
        }).stdout;

        const substrateAddress = parseAddressInternal(
            result
                .split(/\r?\n/)[0] // First line of the output
                .split('Account Substrate address: ')[1], // Substrate address
        );

        const evmAddress = parseEVMAddressInternal(
            result
                .split(/\r?\n/)[1] // Second line of the output
                .split('Associated EVM address: ')[1], // EVM address
        );

        expect(substrateAddress).toEqual(caller.address.toString());
        expect(evmAddress).toEqual(substrateAddressToEvmAddress(caller.address));
    }, 60000);
});
