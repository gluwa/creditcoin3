import { Command } from 'commander';
import { mandatoryProxyOption, proxyTypeOption, delayOption, noInputOption, urlOption } from '../options';
import { setProxyAction, viewProxyAction, removeProxyAction } from './actions';

export function makeProxyCommands() {
    return new Command('proxy')
        .description('Commands for managing the proxy system ')
        .addCommand(makeAddProxyCmd())
        .addCommand(makeListProxyCmd())
        .addCommand(makeRemoveProxyCmd())
        .addOption(noInputOption)
        .addOption(urlOption);
}

export function makeAddProxyCmd() {
    return new Command('add')
        .description('Set the proxy')
        .addOption(mandatoryProxyOption)
        .addOption(proxyTypeOption)
        .addOption(delayOption)
        .addOption(noInputOption)
        .addOption(urlOption)
        .action(setProxyAction);
}

export function makeListProxyCmd() {
    return new Command('list')
        .description('View the current proxy')
        .addOption(noInputOption)
        .addOption(urlOption)
        .action(viewProxyAction);
}

export function makeRemoveProxyCmd() {
    return new Command('remove')
        .description('Remove the current proxy')
        .addOption(mandatoryProxyOption)
        .addOption(delayOption)
        .addOption(noInputOption)
        .addOption(urlOption)
        .action(removeProxyAction);
}
