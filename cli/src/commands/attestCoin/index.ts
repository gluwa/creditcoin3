import { Command } from 'commander';
import { noInputOption, urlOption } from '../options';
import {
    makeAttestCoinAccruedCommand,
    makeAttestCoinAccruedEvmCommand,
    makeAttestCoinClaimNonceCommand,
} from './rewards';

export function makeAttestCoinCommand() {
    const cmd = new Command('attest-coin');
    cmd.description('Attest-coin reward points (runtime + EVM precompile 0x…0fd5)');
    cmd.addCommand(makeAttestCoinAccruedCommand());
    cmd.addCommand(makeAttestCoinAccruedEvmCommand());
    cmd.addCommand(makeAttestCoinClaimNonceCommand());

    cmd.commands.forEach((c) => {
        c.addOption(noInputOption);
        c.addOption(urlOption);
    });
    return cmd;
}
