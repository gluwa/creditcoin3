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
        command.option('--evm-url [evm-url]', 'URL of the EVM RPC endpoint to connect to', 'http://127.0.0.1:9944');
    });
    return cmd;
}
