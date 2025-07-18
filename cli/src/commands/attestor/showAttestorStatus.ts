import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { substrateAddressOption, chainKeyOption } from '../options';

export function makeShowAttestorStatusCommand() {
    const cmd = new Command('show-status');
    cmd.description('Show attestor status for a given address and chain key');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(showAttestorStatus);
    return cmd;
}

async function showAttestorStatus(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;
    const chainKey = options.chain as string;

    const attestor = await api.query.attestation.attestors(chainKey, address);
    if (attestor.isNone) {
        console.log(`Address ${address} is not an attestor`);
        process.exit(0);
    }

    const status = attestor.unwrap().status;
    if (status.isActive) {
        console.log(`Address ${address} status is Active`);
        process.exit(0);
    }
    console.log(`Address ${address} status is Chill`);
    process.exit(0);
}
