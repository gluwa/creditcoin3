// import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';
// import { bond, parseRewardDestination } from '../../lib/staking';
// import { promptContinue, setInteractivity } from '../../lib/interactive';
// import { toCTCString, checkAmount } from '../../lib/balance';

// import { inputOrDefault, parseBoolean, parseChoiceOrExit } from '../../lib/parsing';
// import { initKeyring } from '../../lib/account/keyring';
// import { amountOption, proxyForOption } from '../options';

export async function callRegisterAttestor() {
    const { api } = await newApi();
    const txCall = api.tx.attestation
    .setPayee("5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi");

    await internalSignSendAndWatch(txCall);
}
