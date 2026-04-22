import { Command, OptionValues } from 'commander';
import { substrateAddressOption, chainKeyOption } from '../options';
import {
    getAttestorContractReadOnly,
    substrateAddressToBytes32,
    ATTESTOR_STATUS_ACTIVE,
} from '../../lib/attestor/precompile';

export function makeShowAttestorStatusCommand() {
    const cmd = new Command('show-status');
    cmd.description('Show attestor status for a given address and chain key');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(showAttestorStatus);
    return cmd;
}

async function showAttestorStatus(options: OptionValues) {
    const address = options.substrateAddress as string;
    const chainKey = options.chain as string;
    const attestorId32 = substrateAddressToBytes32(address);

    const contract = getAttestorContractReadOnly(options);
    const attestorInfo = await contract.getAttestor(BigInt(chainKey), attestorId32);

    if (!attestorInfo.exists) {
        console.log(`Address ${address} is not an attestor`);
        process.exit(0);
    }

    if (attestorInfo.status === ATTESTOR_STATUS_ACTIVE) {
        console.log(`Address ${address} status is Active`);
        process.exit(0);
    }
    console.log(`Address ${address} status is Chill`);
    process.exit(0);
}
