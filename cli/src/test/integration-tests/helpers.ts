import { ApiPromise } from '@polkadot/api';
import { mnemonicGenerate } from '@polkadot/util-crypto';
import { newApi } from '../../api';
import { BN } from '../../lib';
import { initEthKeyringPair } from '../../lib/account/keyring';
import { signSendAndWatch } from '../../lib/tx';

export const ALICE_NODE_URL = (global as any).CREDITCOIN_API_URL;
export const BOB_NODE_URL = 'ws://localhost:9945';

export async function fundAccounts(amount: BN) {
    const { api } = await newApi((global as any).CREDITCOIN_API_URL);
    const stash = randomTestAccount();
    const controller = randomTestAccount();
    const tx = await fundAddressesFromSudo(
        [stash.address, controller.address],
        amount
    );
    await signSendAndWatch(tx, api, initAlithKeyring());

    return { stash, controller };
}

export async function fundFromSudo(
    address: string,
    amount: BN,
    url = (global as any).CREDITCOIN_API_URL
) {
    const { api } = await newApi(url);
    const call = api.tx.balances.setBalance(address, amount.toString(), '0');
    const tx = api.tx.sudo.sudo(call);
    return tx;
}

export async function fundAddressesFromSudo(
    addresses: string[],
    amount: BN,
    url = (global as any).CREDITCOIN_API_URL
) {
    const { api } = await newApi(url);
    const txs = addresses.map((address) => {
        const fundTx = api.tx.balances.setBalance(
            address,
            amount.toString(),
            '0'
        );
        return api.tx.sudo.sudo(fundTx);
    });
    const tx = api.tx.utility.batchAll(txs);
    return tx;
}

export async function waitEras(eras: number, api: ApiPromise, force = true) {
    if (force) {
        await forceNewEra(api);
    }
    let eraInfo = await api.derive.session.info();
    let currentEra = eraInfo.currentEra.toNumber();
    const targetEra = currentEra + eras;
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();
    while (currentEra < targetEra) {
        console.log(`Waiting for era ${targetEra}, currently at ${currentEra}`);
        await new Promise((resolve) => setTimeout(resolve, blockTime));
        eraInfo = await api.derive.session.info();
        currentEra = eraInfo.currentEra.toNumber();
    }
}

export async function forceNewEra(api: ApiPromise) {
    const tx = api.tx.staking.forceNewEraAlways();
    const sudoTx = api.tx.sudo.sudo(tx);
    await signSendAndWatch(sudoTx, api, initAlithKeyring());
}

export function randomTestAccount() {
    const secret = mnemonicGenerate();
    const keyring = initEthKeyringPair(secret);
    const address = keyring.address;
    return { secret, keyring, address };
}

export function initAlithKeyring() {
    return initEthKeyringPair(
        'bottom drive obey lake curtain smoke basket hold race lonely fit walk'
    );
}
