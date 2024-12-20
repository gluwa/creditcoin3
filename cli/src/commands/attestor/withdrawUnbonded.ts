import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded-attestor');
    cmd.description('Withdraw unbonded funds from attestor account that become available after calling unregisterAttestor');
    cmd.addOption(proxyForOption);
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const withdrawUnbondedAttestorTx = api.tx.attestation.withdrawUnbonded();

    await requireKeyringHasSufficientFunds(withdrawUnbondedAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(withdrawUnbondedAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
