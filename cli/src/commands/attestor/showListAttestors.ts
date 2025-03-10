import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { substrateAddressOption } from '../options';

export function showListAttestorsCommand() {
    const cmd = new Command('show-list-attestors');
    cmd.description('Show attestor status for a given address and chain key');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option(
        '-c, --chain [chain]',
        'Specify chain key to show attestor status for',
    );
    cmd.action(showListAttestorsAction);
    return cmd;
}

async function showListAttestorsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;
    const chainKey = options.chain as string;

    const attestorsKeys = await api.query.attestation.attestors.keys();
    for (let i = 0; i < attestorsKeys.length; i++) {
        const key = attestorsKeys[i];
        const chain = key.args[0].toString();
        if (chain != chainKey) {
            continue;
        }
        const attestor = await api.query.attestation.attestors(key.args[0], key.args[1]);
        if (attestor.isNone) {
            continue;
        }
        const attestorValue = attestor.unwrap();
        if (attestorValue.stash.toString() == address) {
            console.log(`Address ${key.args[1]} is an attestor for chain ${chainKey}`);
            console.log('');
        }
    }
    process.exit(0);
}
