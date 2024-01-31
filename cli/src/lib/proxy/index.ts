import { PalletProxyProxyDefinition } from '@polkadot/types/lookup';
import { ApiPromise } from '../index';

// Grab all proxies for 'addr', return them as a decoded array
export async function proxiesForAddress(addr: string, api: ApiPromise): Promise<PalletProxyProxyDefinition[]> {
    const [delegates, _] = await api.query.proxy.proxies(addr);
    return delegates.toArray();
}

// Given a list of delegates, check if any match 'addr'
export function addressIsProxy(addr: string, delegates: PalletProxyProxyDefinition[]): boolean {
    return filterProxiesByAddress(addr, delegates).length >= 1;
}

// Given a list of delegates, return only the subset that match the 'addr' supplied
export function filterProxiesByAddress(
    addr: string,
    delegates: PalletProxyProxyDefinition[],
): PalletProxyProxyDefinition[] {
    return delegates.filter((x) => x.delegate.toString() === addr);
}

// Given a list of delegates, check if any have a proxy type that matches 'type'
export function hasProxyType(delegates: PalletProxyProxyDefinition[], type: string): boolean {
    return delegates.find((delegate) => delegate.proxyType.toString() === type) !== undefined;
}
