import { Command } from 'commander';
import { mandatoryProxyOption, proxyTypeOption, noInputOption, urlOption } from '../options';
import { setProxyAction, viewProxyAction, removeProxyAction } from './actions';

export function makeProxyCommands() {
    const cmd = new Command('proxy')
        .description('Commands for managing the proxy system ')
        .addCommand(makeAddProxyCmd())
        .addCommand(makeListProxyCmd())
        .addCommand(makeRemoveProxyCmd());

    cmd.commands.forEach((command) => {
        command.addOption(noInputOption);
        command.addOption(urlOption);
    });

    return cmd;
}

export function makeAddProxyCmd() {
    return new Command('add')
        .description('Add a new proxy')
        .addOption(mandatoryProxyOption)
        .addOption(proxyTypeOption)
        .action(setProxyAction);
}

export function makeListProxyCmd() {
    return new Command('list').description('View a list of proxies and their types').action(viewProxyAction);
}

export function makeRemoveProxyCmd() {
    return new Command('remove')
        .description('Remove all instances of a proxy')
        .addOption(mandatoryProxyOption)
        .action(removeProxyAction);
}
