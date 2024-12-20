import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function attestCommand() {
    const cmd = new Command('attest');
    cmd.description('attest');
    cmd.addOption(proxyForOption);
    cmd.action(attestAction);
    return cmd;
}

async function attestAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chainKey as string;
    const blsPublicKey = options.blsPublicKey as string;
    const proofOfPossession = options.proofOfPossession as string;

    const keyring = await initKeyring(options);

    const attestAttestorTx = api.tx.attestation.attest(chainKey, blsPublicKey, proofOfPossession);

    await requireKeyringHasSufficientFunds(attestAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(attestAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}


