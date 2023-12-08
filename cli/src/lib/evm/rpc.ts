import { OptionValues } from 'commander';

export function getEvmUrl(options: OptionValues): string {
    const url = options.url as string;

    // Check if it is ws or wss and replace with http or https accordingly
    let httpUrl = url;
    if (url.startsWith('ws://')) {
        httpUrl = url.replace('ws://', 'http://');
    } else if (url.startsWith('wss://')) {
        httpUrl = url.replace('wss://', 'https://');
    }

    return httpUrl;
}
