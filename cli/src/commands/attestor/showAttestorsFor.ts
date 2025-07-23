import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { substrateAddressOption, chainKeyOption } from '../options';

export function showAttestorsForCommand() {
    const cmd = new Command('show-attestors-for');
    cmd.description('Show list of attestors for a given address and chain key');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(showAttestorsForAction);
    return cmd;
}

async function showAttestorsForAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;
    const chainKey = BigInt(options.chain);

    const attestorsKeys = await api.query.attestation.attestors.keys();
    for (const [_, key] of attestorsKeys.entries()) {
        const chain = key.args[0].toBigInt();
        if (chain !== chainKey) {
            continue;
        }
        const attestor = await api.query.attestation.attestors(key.args[0], key.args[1]);
        if (attestor.isNone) {
            continue;
        }
        const attestorValue = attestor.unwrap();
        if (attestorValue.stash.toString() === address) {
            console.log(`Address ${key.args[1].toString()} is an attestor for chain ${chainKey}`);
            console.log('');
        }
    }
    process.exit(0);
}
