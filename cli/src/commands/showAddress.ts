import { cryptoWaitReady } from '@polkadot/util-crypto';
import { Command, OptionValues } from 'commander';
import { initCallerKeyring } from '../lib/account/keyring';
import { substrateAddressToEvmAddress } from '../lib/evm/address';

export function makeShowAddressCommand() {
    const cmd = new Command('show-address');
    cmd.description('Show account address');
    cmd.action(showAddressAction);
    return cmd;
}

async function showAddressAction(options: OptionValues) {
    await cryptoWaitReady();

    const caller = await initCallerKeyring(options);

    const evmAddress = substrateAddressToEvmAddress(caller.address);

    console.log('Account Substrate address:', caller.address);
    console.log('Associated EVM address:', evmAddress);

    process.exit(0);
}
