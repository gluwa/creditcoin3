import { cryptoWaitReady } from '@polkadot/util-crypto';
import { ApiPromise, WsProvider } from '@polkadot/api';
import { DispatchError, DispatchResult, EventRecord } from '@polkadot/types/interfaces';

export interface CreditcoinApi {
    api: ApiPromise;
}

const CONNECT_RETRIES = 5;
const CONNECT_TIMEOUT_MS = 10_000;
const CONNECT_BACKOFF_BASE_MS = 500;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Establish a WsProvider whose socket is actually open before it is used.
// WsProvider connects asynchronously, so ApiPromise.create can attempt an RPC
// subscription before the handshake completes, surfacing as
// "WebSocket is not connected". A fresh provider is created per attempt with
// autoConnect left enabled (so Polkadot's default mid-session reconnect is
// preserved on the returned provider) and we await isReady with a bounded
// timeout, retrying with exponential backoff to absorb transient connection
// races. The connected provider is returned to the caller; failed attempts are
// disconnected so no reconnect loop is leaked.
const connectProviderWithRetry = async (wsUrl: string): Promise<WsProvider> => {
    let lastErr: unknown;
    for (let attempt = 1; attempt <= CONNECT_RETRIES; attempt++) {
        // autoConnect enabled (default): keeps mid-session reconnect intact for
        // the provider we ultimately return to the caller.
        const provider = new WsProvider(wsUrl);
        // Bounded timeout that we always clear, so its timer/rejection never
        // outlives the attempt and leaks an unhandled rejection on success.
        let timeoutId: ReturnType<typeof setTimeout> | undefined;
        const timeout = new Promise<never>((_resolve, reject) => {
            timeoutId = setTimeout(
                () => reject(new Error(`WsProvider connect timed out after ${CONNECT_TIMEOUT_MS}ms`)),
                CONNECT_TIMEOUT_MS,
            );
        });
        try {
            await Promise.race([provider.isReady, timeout]);
            return provider;
        } catch (err) {
            lastErr = err;
            // Tear down this attempt's provider so its reconnect loop does not leak.
            await provider.disconnect().catch(() => undefined);
            if (attempt < CONNECT_RETRIES) {
                await sleep(CONNECT_BACKOFF_BASE_MS * 2 ** (attempt - 1));
            }
        } finally {
            // Always release the timer; on success this prevents the pending
            // rejection from surfacing ~10s later on the happy path.
            if (timeoutId !== undefined) {
                clearTimeout(timeoutId);
            }
        }
    }
    throw new Error(
        `Failed to connect to node after ${CONNECT_RETRIES} attempts: ${
            lastErr instanceof Error ? lastErr.message : String(lastErr)
        }`,
    );
};

export const creditcoinApi = async (wsUrl: string, noInitWarn = false): Promise<CreditcoinApi> => {
    const provider = await connectProviderWithRetry(wsUrl);
    try {
        const api = await ApiPromise.create({ provider, noInitWarn });
        await api.isReady;
        return { api };
    } catch (err) {
        // ApiPromise.create failed after a good connect: don't leak the provider.
        await provider.disconnect().catch(() => undefined);
        throw err;
    }
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
