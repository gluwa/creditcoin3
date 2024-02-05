import { OptionValues } from 'commander';
import { newApi } from '../../lib';
import { parseSubstrateAddress } from '../options';
import { initCallerKeyring } from '../../lib/account/keyring';
import { signSendAndWatch, requireEnoughFundsToSend } from '../../lib/tx';

export async function setProxyAction(opts: OptionValues) {
    const { proxyAddr, proxyType, url, delay } = parseSetProxyOptions(opts);

    const { api } = await newApi(url);
    const callerKeyring = await initCallerKeyring(opts);

    const call = api.tx.proxy.addProxy(proxyAddr, proxyType, delay);
    await requireEnoughFundsToSend(call, callerKeyring.address, api);
    const result = await signSendAndWatch(call, api, callerKeyring);

    console.log(result);
    process.exit(0);
}

function parseSetProxyOptions(opts: OptionValues) {
    const proxyAddr = parseSubstrateAddress(opts.proxy);
    const proxyType = opts.type;
    const url = opts.url;
    const delay = opts.delay ? opts.delay : 0;

    return { proxyAddr, proxyType, url, delay };
}

export async function viewProxyAction(opts: OptionValues) {
    const { api } = await newApi(opts.url);

    const callerKeyring = await initCallerKeyring(opts);
    const callerAddress = callerKeyring.address;
    const callerProxy = await api.query.proxy.proxies(callerAddress);

    console.log(callerProxy.toJSON());
    process.exit(0);
}

export async function removeProxyAction(opts: OptionValues) {
    const { api } = await newApi(opts.url);

    const callerKeyring = await initCallerKeyring(opts);
    const callerAddress = callerKeyring.address;

    let [defArray, _] = await await api.query.proxy.proxies(callerAddress);
    if (defArray.toArray().length === 0) {
        console.log(`ERROR: No proxies has been set for ${callerAddress}`);
        process.exit(1);
    }

    const proxy = opts.proxy;
    const type = opts.type;
    const delay = opts.delay ? opts.delay : 0;

    const call = api.tx.proxy.removeProxy(proxy, type, delay);
    await requireEnoughFundsToSend(call, callerAddress, api);
    const result = await signSendAndWatch(call, api, callerKeyring);

    console.log(result);
    process.exit(0);
}
