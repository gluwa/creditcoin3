import { OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initKeyring } from '../../lib/account/keyring';
import { signSendAndWatchCcKeyring, requireKeyringHasSufficientFunds } from '../../lib/tx';
import { addressIsAlreadyProxy, addressIsProxy, filterProxiesByAddress, proxiesForAddress } from '../../lib/proxy';

export async function setProxyAction(options: OptionValues) {
    const { url } = options;
    const proxyAddr = options.proxy;
    const proxyType = options.type;

    const { api } = await newApi(url as string);
    const callerKeyring = await initKeyring(options);
    // note: no proxy used here, access .pair.address directly
    const callerAddress = callerKeyring.pair.address;

    const existingProxiesForAddress = await proxiesForAddress(callerAddress, api);
    if (existingProxiesForAddress.length >= 1) {
        console.error(`ERROR: There is already an existing proxy set for ${callerAddress}`);
        process.exit(1);
    }

    if (await addressIsAlreadyProxy(proxyAddr, api)) {
        console.error(`ERROR: The proxy ${proxyAddr} is already in use with another validator`);
        process.exit(2);
    }

    const call = api.tx.proxy.addProxy(proxyAddr, proxyType, 0);
    await requireKeyringHasSufficientFunds(call, callerKeyring, api);
    const result = await signSendAndWatchCcKeyring(call, api, callerKeyring);

    console.log(result);
    process.exit(result.status);
}

export async function viewProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);
    const callerKeyring = await initKeyring(options);
    // note: no proxy used here, access .pair.address directly
    const callerAddress = callerKeyring.pair.address;
    const proxies = await proxiesForAddress(callerAddress, api);

    if (proxies.length === 0) {
        console.log(`No proxies for address ${callerAddress}`);
        process.exit(0);
    }
    console.log(`Proxies for address ${callerAddress}`);
    for (const p of proxies) {
        console.log(p.toString());
    }
    process.exit(0);
}

export async function removeProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const callerKeyring = await initKeyring(options);
    // note: no proxy used here, access .pair.address directly
    const callerAddress = callerKeyring.pair.address;

    const proxies = await proxiesForAddress(callerAddress, api);
    if (proxies.length === 0) {
        console.error(`ERROR: No proxies have been set for ${callerAddress}`);
        process.exit(1);
    }

    const existingProxy = filterProxiesByAddress(options.proxy, proxies);
    if (!addressIsProxy(options.proxy, existingProxy)) {
        console.error(`ERROR: ${options.proxy as string} is not a proxy for ${callerAddress}`);
        process.exit(1);
    }

    const success: string[] = [];
    const fails: string[] = [];
    const proxy = options.proxy as string; // proxy and type are mandatory it is safe to just grab them

    console.log(`${existingProxy.length} proxies found`);

    for (const p of existingProxy) {
        const type = p.proxyType; // proxy is validated as a substrate address and type is also validated prior to us using it here
        const delay = p.delay;
        const call = api.tx.proxy.removeProxy(proxy, type, delay);

        try {
            await requireKeyringHasSufficientFunds(call, callerKeyring, api);
            console.log(`Removing proxy ${proxy} with type ${type.toString()}`);
            const result = await signSendAndWatchCcKeyring(call, api, callerKeyring);
            console.log(result);
            success.push(p.toString());
        } catch (e) {
            console.log(`ERROR removing proxy ${proxy} with type ${type.toString()}: ${e as string}`);
            fails.push(p.toString());
        }
    }

    console.log(`${success.length} proxies removed`);
    console.log(success);
    if (fails.length > 0) {
        console.log(`${fails.length} proxies failed to be removed`);
        console.log(fails);
    }
    process.exit(fails.length);
}
