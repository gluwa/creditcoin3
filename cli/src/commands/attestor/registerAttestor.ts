import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeRegisterAttestorCommand() {
    const cmd = new Command('register');
    cmd.description('Register an attestor and bond funds from a stash account');
    cmd.addOption(proxyForOption);
    cmd.option('-a, --attestor [attestor]', 'Specify the attestor account to register');
    cmd.option('-c, --chain [chain]', 'Specify chain key to register attestor for');
    cmd.action(registerAttestorAction);
    return cmd;
}

async function registerAttestorAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chain as string;
    const attestor = options.attestor as string;

    const keyring = await initKeyring(options);

    const registerAttestorTx = api.tx.attestation.registerAttestor(chainKey, attestor);

    await requireKeyringHasSufficientFunds(registerAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(registerAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
