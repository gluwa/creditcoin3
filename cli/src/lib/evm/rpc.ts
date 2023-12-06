import { OptionValues } from 'commander';

export function getEvmUrl(options: OptionValues): string {
    const url = options.url as string;
    const httpUrl = url.replace('ws://', 'http://');
    return httpUrl;
}
