import { OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';
import { signSendAndWatch, requireEnoughFundsToSend } from '../../lib/tx';
import { addressIsProxy, filterProxiesByAddress, proxiesForAddress } from '../../lib/proxy';

export async function setProxyAction(options: OptionValues) {
    const { url, delay } = options;
    const proxyAddr = options.proxy;
    const proxyType = options.type;

    const { api } = await newApi(url as string);
    const callerKeyring = await initCallerKeyring(options);

    const call = api.tx.proxy.addProxy(proxyAddr, proxyType, delay);
    await requireEnoughFundsToSend(call, callerKeyring.address, api);
    const result = await signSendAndWatch(call, api, callerKeyring);

    console.log(result);
    process.exit(result.status);
}

export async function viewProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);
    const callerKeyring = await initCallerKeyring(options);
    const callerAddress = callerKeyring.address;
    const proxies = await proxiesForAddress(callerAddress, api);

    if (proxies.length === 0) {
        console.log(`No proxies for address ${callerAddress}`);
        process.exit(0);
    }
    console.log(`Proxies for address ${callerKeyring.address}`);
    for (const p of proxies) {
        console.log(p.toString());
    }
    process.exit(0);
}

export async function removeProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    // force=true means we get back the keyring even though we enabled the --proxy flag
    const callerKeyring = await initCallerKeyring(options);
    const callerAddress = callerKeyring.address;

    const proxies = await proxiesForAddress(callerAddress, api);
    if (proxies.length === 0) {
        console.log(`ERROR: No proxies have been set for ${callerAddress}`);
        process.exit(1);
    }

    const existingProxy = filterProxiesByAddress(options.proxy, proxies);
    if (!addressIsProxy(options.proxy, existingProxy)) {
        console.log(`ERROR: ${options.proxy as string} is not a proxy for ${callerAddress}`);
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
            await requireEnoughFundsToSend(call, callerAddress, api);
            console.log(`Removing proxy ${proxy} with type ${type.toString()}`);
            const result = await signSendAndWatch(call, api, callerKeyring);
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
