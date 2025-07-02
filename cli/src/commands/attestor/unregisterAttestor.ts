import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption, attestorAddressOption, chainKeyOption } from '../options';

export function makeUnregisterAttestorCommand() {
    const cmd = new Command('unregister');
    cmd.description('Unregister attestor and unbond funds from a stash account');
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.addOption(proxyForOption);
    cmd.action(unregisterAttestorAction);
    return cmd;
}

async function unregisterAttestorAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chain as string;
    const attestor = options.attestor as string;

    const keyring = await initKeyring(options);

    const attestorStatus = await api.query.attestation.attestors(chainKey, attestor);
    if (attestorStatus.isNone) {
        console.log(`Address ${attestor} is not an attestor`);
        process.exit(1);
    }

    const status = attestorStatus.unwrap().status;
    if (status.isActive) {
        console.log(`Address ${attestor} status is Active. Please chill the attestor first`);
        process.exit(1);
    }
    console.log(`Address ${attestor} status is Chill`);
    console.log(`Calling unregister attestor extrinsic for ${attestor} on chain ${chainKey}`);

    const unregisterAttestorTx = api.tx.attestation.unregisterAttestor(chainKey, attestor);

    await requireKeyringHasSufficientFunds(unregisterAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(unregisterAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
