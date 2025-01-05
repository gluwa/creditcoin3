// import { ISubmittableResult } from '@polkadot/types/types';

// import { SubmittableExtrinsic } from '@polkadot/api/types';
// import { AccountBalance, getBalance, toCTCString } from './balance';
// import { ApiPromise, BN, KeyringPair } from '.';
// import { CcKeyring, ProxyKeyring, delegateAddress, isProxy } from './account/keyring';

// import { DispatchError, DispatchResult, EventRecord } from '@polkadot/types/interfaces';

import { newApi } from '../lib/api';

import { web3Accounts, web3Enable, web3FromAddress } from '@polkadot/extension-dapp';
import { InjectedExtension } from '@polkadot/extension-inject/types';
const { ApiPromise, WsProvider } = require('@polkadot/api');

// export async function internalSignSendAndWatch2(){
//     let api = await newApi();
// }

// import { ApiPromise, WsProvider } from '@polkadot/api';

// WARNING: this function should not be used directly, use signSendAndWatchCcKeyring() instead!
export async function internalSignSendAndWatch(
    txCall: any, // Accepts any type for tx
) {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    const allAccounts = await web3Accounts();

    const account = allAccounts[0];

    const accountStr = account.address;

    //iterates over all accounts and console logs the address
    allAccounts.forEach(({ address, meta }) => {
        console.log(`Address: ${address}, meta: ${meta.name}`);
    });
    

    // const SENDER = '5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi';
    const SENDER = accountStr;

    // finds an injector for an address
    const injector = await web3FromAddress(SENDER); 

    // const wsProvider = new WsProvider('ws://127.0.0.1:9944');
    // const api = await ApiPromise.create({ provider: wsProvider });

    // const api = newApi();

    // const metadata = await api.runtimeMetadata.toHuman();
    // console.log('Metadata:', metadata);

    // api.tx.balances
    // .transfer('5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi', 1000000000000000)
    // .signAndSend(SENDER, { signer: injector.signer }, () => { console.log("send balance") });

    txCall
    .signAndSend(SENDER, { signer: injector.signer }, () => { console.log("sent tx") });

    
}

//this fn should accept a InjectedExtension object as an argument
export async function internalSignSendAndWatchBySender(
    txCall: any, // Accepts any type for tx
    // injector: InjectedExtension // Accepts any type for the API object
    sender: string
) {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    // const allAccounts = await web3Accounts();

    // const account = allAccounts[0];
    
    // const accountStr = account.address;

    // //iterates over all accounts and console logs the address
    // allAccounts.forEach(({ address, meta }) => {
    //     console.log(`Address: ${address}, meta: ${meta.name}`);
    // });
    

    // const SENDER = '5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi';
    const SENDER = sender;

    // finds an injector for an address
    const injector = await web3FromAddress(SENDER); 

    // const accounts_list = await injector.accounts.get(true);
    // const first = accounts_list[0];
    // const first_address = first.address

    //take and address of injected account

    // const wsProvider = new WsProvider('ws://127.0.0.1:9944');
    // const api = await ApiPromise.create({ provider: wsProvider });

    // const api = newApi();

    // const metadata = await api.runtimeMetadata.toHuman();
    // console.log('Metadata:', metadata);

    // api.tx.balances
    // .transfer('5DkZod7NZdZP21Xij14Qh21hyx2NnU95p6TcscGxByTwuyxi', 1000000000000000)
    // .signAndSend(SENDER, { signer: injector.signer }, () => { console.log("send balance") });

    txCall
    .signAndSend(SENDER, { signer: injector.signer }, () => { console.log("tx sent") });
}

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


export async function enableWeb3AndGetListOfAccounts(): Promise<string[]> {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    const allAccounts = await web3Accounts();

    // Collect all accounts into a list of strings
    const accounts_list: string[] = allAccounts.map(({ address }) => address);

    // Return the list of accounts
    return accounts_list;
}


