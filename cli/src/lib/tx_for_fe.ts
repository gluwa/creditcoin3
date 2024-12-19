// import { ISubmittableResult } from '@polkadot/types/types';

// import { SubmittableExtrinsic } from '@polkadot/api/types';
// import { AccountBalance, getBalance, toCTCString } from './balance';
// import { ApiPromise, BN, KeyringPair } from '.';
// import { CcKeyring, ProxyKeyring, delegateAddress, isProxy } from './account/keyring';

// import { DispatchError, DispatchResult, EventRecord } from '@polkadot/types/interfaces';

import { newApi } from '../lib';

import { web3Accounts, web3Enable, web3FromAddress } from '@polkadot/extension-dapp';
const { ApiPromise, WsProvider } = require('@polkadot/api');

export async function internalSignSendAndWatch2(){
    let api = await newApi();
}

// import { ApiPromise, WsProvider } from '@polkadot/api';

// WARNING: this function should not be used directly, use signSendAndWatchCcKeyring() instead!
export async function internalSignSendAndWatch(
    // tx: SubmittableExtrinsic<'promise', ISubmittableResult>,
    // api: ApiPromise,
    // signer: KeyringPair,
    txCall: any, // Accepts any type for tx
    // api2: any // Accepts any type for the API object
) {
    const allInjected = await web3Enable('attestor creditcoin web3 js app');

    const allAccounts = await web3Accounts();

    const account = allAccounts[0];
    // account to string
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
