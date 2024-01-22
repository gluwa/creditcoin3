import { OptionValues } from "commander";

export function parseSetProxyOptions(opts: OptionValues) {
    const proxyAddr = opts.proxy;
    const proxyType = opts.type;
    const url = opts.url;
    const delay = opts.delay ? opts.delay : 0;

    return { proxyAddr, proxyType, url, delay };
}
