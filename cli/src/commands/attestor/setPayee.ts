import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { payeeOption, proxyForOption } from '../options';

export function setPayeeCommand() {
    const cmd = new Command('set-payee');
    cmd.description('Set payee address for attestor, which will receive rewards on claim rewards');
    cmd.addOption(proxyForOption);
    cmd.addOption(payeeOption.makeOptionMandatory());
    cmd.action(setPayeeAction);
    return cmd;
}

async function setPayeeAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const { payee } = options;

    const setPayeeAttestorTx = api.tx.attestation.setPayee(payee);

    await requireKeyringHasSufficientFunds(setPayeeAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(setPayeeAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
