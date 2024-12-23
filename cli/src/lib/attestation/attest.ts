// import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';
// import { bond, parseRewardDestination } from '../../lib/staking';
// import { promptContinue, setInteractivity } from '../../lib/interactive';
// import { toCTCString, checkAmount } from '../../lib/balance';

// import { inputOrDefault, parseBoolean, parseChoiceOrExit } from '../../lib/parsing';
// import { initKeyring } from '../../lib/account/keyring';
// import { amountOption, proxyForOption } from '../options';

export type OptionValues = Record<string, any>;

export async function callRegisterAttestor(options: OptionValues) {

    const chainKey = options.chainKey as string;
    const blsPublicKey = options.blsPublicKey as string;
    const proofOfPossession = options.proofOfPossession as string;

    const { api } = await newApi();
    const txCall = api.tx.attestation
    .attest(chainKey, blsPublicKey, proofOfPossession);

    await internalSignSendAndWatch(txCall);
}
