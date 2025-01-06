
import { newApi } from '../lib/api';

import { web3Accounts, web3Enable, web3FromAddress } from '@polkadot/extension-dapp';
import { InjectedExtension } from '@polkadot/extension-inject/types';
const { ApiPromise, WsProvider } = require('@polkadot/api');

//this fn should accept a InjectedExtension object as an argument
export async function internalSignSendAndWatchBySender(
    txCall: any, // Accepts any type for tx
    // injector: InjectedExtension // Accepts any type for the API object
    sender: string
) {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    const SENDER = sender;

    const injector = await web3FromAddress(SENDER); 

    txCall
    .signAndSend(SENDER, { signer: injector.signer }, () => { console.log("tx sent") });
}

export async function enableWeb3AndGetListOfAccounts(): Promise<string[]> {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    const allAccounts = await web3Accounts();

    const accounts_list: string[] = allAccounts.map(({ address }) => address);

    return accounts_list;
}


