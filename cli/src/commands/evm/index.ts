import { Command } from 'commander';
import { makeEvmFundCommand } from './fund';
import { makeEvmWithdrawCommand } from './withdraw';
import { makeEvmSendCommand } from './send';
import { makeEvmBalanceCommand } from './balance';

export function makeEvmCommand() {
    const cmd = new Command('evm');
    cmd.description('Interact with the EVM side of Creditcoin3');
    cmd.addCommand(makeEvmBalanceCommand());
    cmd.addCommand(makeEvmFundCommand());
    cmd.addCommand(makeEvmSendCommand());
    cmd.addCommand(makeEvmWithdrawCommand());

    cmd.commands.forEach((command) => {
        command.option('-u, --url [url]', 'URL for the Substrate node', 'ws://127.0.0.1:9944');
        command.option('--no-input', 'Disable interactive prompts');
    });
    return cmd;
}
