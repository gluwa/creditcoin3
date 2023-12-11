import { OptionValues } from 'commander';

export function getEvmUrl(options: OptionValues): string {
    const url = options.url as string;

    if (!url) {
        throw new Error('EVM URL is required');
    }

    const httpUrl = convertWsToHttp(url);

    return httpUrl;
}

export function convertWsToHttp(url: string): string {
    let httpUrl = url;
    if (url.startsWith('ws://')) {
        httpUrl = url.replace('ws://', 'http://');
    } else if (url.startsWith('wss://')) {
        httpUrl = url.replace('wss://', 'https://');
    }

    return httpUrl;
}