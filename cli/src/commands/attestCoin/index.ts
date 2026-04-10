import { Command } from 'commander';
import { noInputOption, urlOption } from '../options';
import { makeAttestCoinAccruedCommand, makeAttestCoinAccruedEvmCommand, makeAttestCoinClaimCommand } from './rewards';

export function makeAttestCoinCommand() {
    const cmd = new Command('attest-coin');
    cmd.description('Attest-coin reward points (runtime + EVM precompile 0x…0fd5)');
    cmd.addCommand(makeAttestCoinAccruedCommand());
    cmd.addCommand(makeAttestCoinAccruedEvmCommand());
    cmd.addCommand(makeAttestCoinClaimCommand());

    cmd.commands.forEach((c) => {
        c.addOption(noInputOption);
        c.addOption(urlOption);
    });
    return cmd;
}
