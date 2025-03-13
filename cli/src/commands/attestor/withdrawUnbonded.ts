import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeAttestorWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded-attestor');
    cmd.description(
        'Withdraw unbonded funds from attestor account that become available after calling unregisterAttestor',
    );
    cmd.addOption(proxyForOption);
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const ledger = await api.query.attestation.ledger(keyring.pair.address);
    if (ledger.isNone) {
        console.log(`No unbonded funds to withdraw for address ${keyring.pair.address}`);
        process.exit(0);
    }
    const ledgerValue = ledger.unwrap();

    const currentEra = await api.query.staking.currentEra();
    if (currentEra.isNone) {
        console.log('Current era is not available');
        process.exit(0);
    }
    const currentEraValue = currentEra.unwrap();

    let canWithdraw = 0;
    for (const unlocking of ledgerValue.unlocking) {
        if (unlocking.era.toNumber() <= currentEraValue.toNumber()) {
            canWithdraw += unlocking.value.toNumber();
        }
    }

    if (canWithdraw === 0) {
        console.log('No unbonded funds to withdraw');
        process.exit(0);
    }

    console.log(`Unbonded funds available to withdraw: ${canWithdraw}`);

    const withdrawUnbondedAttestorTx = api.tx.attestation.withdrawUnbonded();

    await requireKeyringHasSufficientFunds(withdrawUnbondedAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(withdrawUnbondedAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
