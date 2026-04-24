import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { evmAddressOption, chainKeyOption } from '../options';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';

export function showAttestorsForCommand() {
    const cmd = new Command('show-attestors-for');
    cmd.description('Show list of attestors for a given stash (EVM address) and chain key');
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.action(showAttestorsForAction);
    return cmd;
}

async function showAttestorsForAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    // The attestor-stash precompile records the stash as the AccountId derived
    // from the caller's EVM address via HashedAddressMapping. The user-facing
    // input is therefore the EVM `0x…` address; we convert it to that mapped
    // AccountId before looking up stored attestors.
    const evmAddress = options.evmAddress as string;
    const stashAccountId = evmAddressToSubstrateAddress(evmAddress);
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
        if (attestorValue.stash.toString() === stashAccountId) {
            console.log(`Address ${key.args[1].toString()} is an attestor for chain ${chainKey}`);
            console.log('');
        }
    }
    process.exit(0);
}
