import { Command, Option, OptionValues } from 'commander';
import { proxyOption, proxyTypeOption, delayOption } from '../options';
import { setProxyAction, viewProxyAction, removeProxyAction } from './actions';
import { ProxyTypes } from './types';

export function makeProxyCommands() {
    return new Command('proxy')
        .description('Commands for managing the proxy system ')
        .addCommand(makeAddProxyCmd())
        .addCommand(makeListProxyCmd())
        .addCommand(makeRemoveProxyCmd());
}

export function makeAddProxyCmd() {
    return new Command('add')
        .description('Set the proxy')
        .addOption(proxyOption.makeOptionMandatory())
        .addOption(proxyTypeOption.choices(ProxyTypes).makeOptionMandatory())
        .addOption(delayOption)
        .action(setProxyAction);
}

export function makeListProxyCmd() {
    return new Command('list').description('View the current proxy').action(viewProxyAction);
}

export function makeRemoveProxyCmd() {
    return new Command('remove')
        .description('Remove the current proxy')
        .addOption(proxyOption.makeOptionMandatory())
        .addOption(proxyTypeOption.choices(ProxyTypes).makeOptionMandatory())
        .addOption(delayOption)
        .action(removeProxyAction);
}
