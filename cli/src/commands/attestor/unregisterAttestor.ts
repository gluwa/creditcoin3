import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeUnregisterAttestorCommand() {
    const cmd = new Command('unregister-attestor');
    cmd.description('Unregister attestor and unbond funds from a stash account');
    cmd.option(
        '-a, --attestor [attestor]',
        'Specify attestor account to unregister',
    );
    cmd.option(
        '-c, --chain [chain]',
        'Specify chain key to unregister attestor for',
    );
    cmd.addOption(proxyForOption);
    cmd.action(unregisterAttestorAction);
    return cmd;
}

async function unregisterAttestorAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chain as string;
    const attestor = options.attestor as string;

    const keyring = await initKeyring(options);

    const unregisterAttestorTx = api.tx.attestation.unregisterAttestor(chainKey, attestor);

    await requireKeyringHasSufficientFunds(unregisterAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(unregisterAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
