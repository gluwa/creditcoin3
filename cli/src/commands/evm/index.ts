import { Command } from 'commander';
import { makeEvmFundCommand } from './fund';
import { makeEvmWithdrawCommand } from './withdraw';
import { makeEvmBalanceCommand } from './balance';
import { noInputOption, urlOption } from '../options';

export function makeEvmCommand() {
    const cmd = new Command('evm');
    cmd.description('Interact with the EVM side of Creditcoin3');
    cmd.addCommand(makeEvmBalanceCommand());
    cmd.addCommand(makeEvmFundCommand());
    cmd.addCommand(makeEvmWithdrawCommand());

    cmd.commands.forEach((command) => {
        command.addOption(noInputOption);
        command.addOption(urlOption);
    });
    return cmd;
}
