import { cryptoWaitReady } from '@polkadot/util-crypto';
import { ApiPromise, WsProvider } from '@polkadot/api';
import { DispatchError, DispatchResult, EventRecord } from '@polkadot/types/interfaces';

export interface CreditcoinApi {
    api: ApiPromise;
}

export const creditcoinApi = async (wsUrl: string, noInitWarn = false): Promise<CreditcoinApi> => {
    const provider = new WsProvider(wsUrl);
    const api = await ApiPromise.create({ provider, noInitWarn });
    await api.isReady;

    return { api };
};

// Create new API instance
export async function newApi(url = 'ws://127.0.0.1:9944') {
    const ccApi = await creditcoinApi(url.trim(), true);
    await cryptoWaitReady();
    return ccApi;
}

// helper functions for transactions subscriptions
const isDispatchError = (instance: any): instance is DispatchResult => {
    return (instance as DispatchResult) !== undefined;
};

export const expectNoEventError = (api: ApiPromise, eventRecord: EventRecord) => {
    const {
        event: { data },
    } = eventRecord;
    if (data[0] && isDispatchError(data[0])) {
        const dispatchResult = data[0];
        if (dispatchResult.isErr) {
            expectNoDispatchError(api, dispatchResult.asErr);
        }
    }
};

const parseModuleError = (api: ApiPromise, dispatchError: DispatchError): string => {
    const decoded = api.registry.findMetaError(dispatchError.asModule);
    const { docs, name, section } = decoded;
    return `${section}.${name}: ${docs.join(' ')}`;
};

export const expectNoDispatchError = (api: ApiPromise, dispatchError?: DispatchError): void => {
    if (dispatchError) {
        const errString = dispatchError.isModule ? parseModuleError(api, dispatchError) : dispatchError.toString();
        throw new Error(errString);
    }
};
