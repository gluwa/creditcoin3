import { cryptoWaitReady } from '@polkadot/util-crypto';
import { Command, OptionValues } from 'commander';
import { initKeyring } from '../lib/account/keyring';
import { substrateAddressToEvmAddress } from '../lib/evm/address';

export function makeShowAddressCommand() {
    const cmd = new Command('show-address');
    cmd.description('Show account address');
    cmd.action(showAddressAction);
    return cmd;
}

async function showAddressAction(options: OptionValues) {
    await cryptoWaitReady();

    const caller = await initKeyring(options);
    // note: no proxy used here, access .pair.address directly
    const evmAddress = substrateAddressToEvmAddress(caller.pair.address);

    console.log('Account Substrate address:', caller.pair.address);
    console.log('Associated EVM address:', evmAddress);

    process.exit(0);
}
