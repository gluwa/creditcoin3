import { Command } from 'commander';
import { makeEvmFundCommand } from './fund';
import { makeEvmWithdrawCommand } from './withdraw';
import { makeEvmSendCommand } from './send';

export function makeEvmCommand() {
    const cmd = new Command('evm');
    cmd.description('Interact with the EVM side of Creditcoin3');
    cmd.addCommand(makeEvmFundCommand());
    cmd.addCommand(makeEvmSendCommand());
    cmd.addCommand(makeEvmWithdrawCommand());

    cmd.commands.forEach((command) => {
        command.option('--no-input', 'Disable interactive prompts');
    });
    return cmd;
}
