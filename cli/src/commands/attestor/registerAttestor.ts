import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeRegisterAttestorCommand() {
    const cmd = new Command('register-attestor');
    cmd.description('Register attestor and bond funds from a stash account');
    cmd.addOption(proxyForOption);
    cmd.action(registerAttestorAction);
    return cmd;
}

async function registerAttestorAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chainKey as string;

    const keyring = await initKeyring(options);

    const validatorAddr = delegateAddress(keyring);

    const registerAttestorTx = api.tx.attestation.registerAttestor(chainKey, validatorAddr);

    await requireKeyringHasSufficientFunds(registerAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(registerAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
