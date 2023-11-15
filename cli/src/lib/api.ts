import { cryptoWaitReady } from '@polkadot/util-crypto';
import { ApiPromise, WsProvider } from '@polkadot/api';
export interface CreditcoinApi {
    api: ApiPromise;
}

export const creditcoinApi = async (wsUrl: string, noInitWarn = false): Promise<CreditcoinApi> => {
    const provider = new WsProvider(wsUrl);
    const api = await ApiPromise.create({ provider, noInitWarn });

    return { api };
};

// Create new API instance
export async function newApi(url = 'ws://localhost:9944') {
    const ccApi = await creditcoinApi(url.trim(), true);
    await cryptoWaitReady();
    return ccApi;
}
