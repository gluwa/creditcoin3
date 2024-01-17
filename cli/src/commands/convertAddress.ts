import { Command, OptionValues } from 'commander';
import { substrateAddressToEvmAddress, evmAddressToSubstrateAddress } from '../lib/evm/address';
import { ValidatedAddress, unknownAddressOption } from './options';

export function makeConvertAddressCommand() {
    const cmd = new Command('convert-address');
    cmd.description('Get the associated EVM/Substrate address for a Substrate or EVM account');
    cmd.addOption(unknownAddressOption.makeOptionMandatory());
    cmd.action(convertAddressAction);
    return cmd;
}

function convertAddressAction(options: OptionValues) {
    const address = options.address as ValidatedAddress;
    const type = address.type;
    if (type === 'EVM') {
        console.log(`AssociatedSubstrate address: ${evmAddressToSubstrateAddress(address.address)}`);
    } else if (type === 'Substrate') {
        console.log(`Associated EVM address: ${substrateAddressToEvmAddress(address.address)}`);
    } else {
        console.error('Invalid address type');
        process.exit(1);
    }
    process.exit(0);
}
