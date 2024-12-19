// import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { internalSignSendAndWatch } from '../lib/tx_for_fe';
// import { bond, parseRewardDestination } from '../../lib/staking';
// import { promptContinue, setInteractivity } from '../../lib/interactive';
// import { toCTCString, checkAmount } from '../../lib/balance';

// import { inputOrDefault, parseBoolean, parseChoiceOrExit } from '../../lib/parsing';
// import { initKeyring } from '../../lib/account/keyring';
// import { amountOption, proxyForOption } from '../options';

import { web3Accounts, web3Enable } from '@polkadot/extension-dapp';

export async function callTransferAdvanced() {
    const { api } = await newApi();

    // const allInjected = await web3Enable('attestor creditcoin web3 js app');

    // const allAccounts = await web3Accounts();
    // //take the first account that we have access to
    // const account = allAccounts[0];
    // // account to string
    // const accountStr = account.address;

    const txCall = api.tx.balances
    .transfer("5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi", 1000000000000000);

    await internalSignSendAndWatch(txCall);
}
