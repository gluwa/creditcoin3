import { ApiPromise, WsProvider } from '@polkadot/api';
import type { CreditcoinApi } from './types';

export const creditcoinApi = async (wsUrl: string, noInitWarn = false): Promise<CreditcoinApi> => {
    const provider = new WsProvider(wsUrl);
    const api = await ApiPromise.create({ provider, noInitWarn });

    return { api };
};
