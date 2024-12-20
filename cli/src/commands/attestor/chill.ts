import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeChillCommand() {
    const cmd = new Command('chill-attestor');
    cmd.description('Chill attestor');
    cmd.addOption(proxyForOption);
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chainKey as string;

    const keyring = await initKeyring(options);

    const chillAttestorTx = api.tx.attestation.chill(chainKey);

    await requireKeyringHasSufficientFunds(chillAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(chillAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
