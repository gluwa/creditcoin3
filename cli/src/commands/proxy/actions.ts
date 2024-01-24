import { OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';
import { signSendAndWatch, requireEnoughFundsToSend } from '../../lib/tx';
import { parseSetProxyOptions } from './utils';

export async function setProxyAction(options: OptionValues) {
    const { proxyAddr, proxyType, url, delay } = parseSetProxyOptions(options);

    const { api } = await newApi(url);
    const callerKeyring = await initCallerKeyring(options, true);

    if (!callerKeyring) {
        throw new Error('Keyring not initialized and not using a proxy');
    }

    const call = api.tx.proxy.addProxy(proxyAddr, proxyType, delay);
    await requireEnoughFundsToSend(call, callerKeyring.address, api);
    const result = await signSendAndWatch(call, api, callerKeyring);

    console.log(result);
    process.exit(0);
}

export async function viewProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const callerKeyring = await initCallerKeyring(options, true);
    if (!callerKeyring) {
        throw new Error('Keyring not initialized and not using a proxy');
    }

    const callerAddress = callerKeyring.address;
    const callerProxy = await api.query.proxy.proxies(callerAddress);

    const [defArray, _] = callerProxy;
    if (defArray.toArray().length === 0) {
        console.log(`No proxies for address ${callerAddress}`);
        process.exit(0);
    }
    console.log(`Proxies for address ${callerKeyring.address}`);
    console.log(callerProxy.toJSON());
    process.exit(0);
}

export async function removeProxyAction(options: OptionValues) {
    const { api } = await newApi(options.url);

    // force=true means we get back the keyring even though we enabled the --proxy flag
    const callerKeyring = await initCallerKeyring(options, true);
    if (!callerKeyring) {
        throw new Error('Keyring not initialized and not using a proxy');
    }
    const callerAddress = callerKeyring.address;

    const [defArray, _] = await api.query.proxy.proxies(callerAddress);
    if (defArray.toArray().length === 0) {
        console.log(`ERROR: No proxies have been set for ${callerAddress}`);
        process.exit(1);
    }

    const existingProxy = defArray.toArray().filter((x) => x.delegate.toString() === options.proxy);
    if (existingProxy.length === 0) {
        console.log(`ERROR: ${options.proxy as string} is not a proxy for ${callerAddress}`);
        process.exit(1);
    }

    console.log(`${existingProxy.length} proxies found`);
    for (const p of existingProxy) {
        const proxy = options.proxy as string; // proxy and type are mandatory it is safe to just grab them
        const type = p.proxyType; // proxy is validated as a substrate address and type is also validated prior to us using it here
        const delay = options.delay ? options.delay : 0;
        const call = api.tx.proxy.removeProxy(proxy, type, delay);
        await requireEnoughFundsToSend(call, callerAddress, api);
        console.log(`Removing proxy ${proxy} with type ${type.toString()}`);
        const result = await signSendAndWatch(call, api, callerKeyring);
        console.log(result);
    }

    console.log(`${existingProxy.length} Proxies removed`);
    process.exit(0);
}
