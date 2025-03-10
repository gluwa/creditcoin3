import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';
import { substrateAddressOption } from '../options';

export function makeShowAttestorStatusCommand() {
    const cmd = new Command('show-attestor-status');
    cmd.description('Show attestor status for a given address and chain key');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option(
        '-c, --chain [chain]',
        'Specify chain key to show attestor status for',
    );
    cmd.action(showAttestorStatus);
    return cmd;
}

async function showAttestorStatus(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;
    const chainKey = options.chain as string;

    const activeAttestors = await api.query.attestation.activeAttestors(chainKey);
    for (let i = 0; i < activeAttestors.length; i++) {
        if (activeAttestors[i].toString() === address) {
            console.log(`Address ${address} status is Elected`);
            process.exit(0);
        }
    }

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
