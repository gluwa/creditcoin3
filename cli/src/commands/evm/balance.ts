import { Command, OptionValues } from 'commander';
import { parseEVMAddressOrExit } from '../../lib/parsing';
import { getEvmUrl } from '../../lib/evm/rpc';
import { getEVMBalanceOf, logEVMBalance } from '../../lib/evm/balance';

export function makeEvmBalanceCommand() {
    const cmd = new Command('balance');
    cmd.description('Show balance of an EVM account');
    cmd.argument('<address>', 'Address to check balance of');
    cmd.option('--json', 'Output as JSON');
    cmd.action(evmBalanceAction);
    return cmd;
}

async function evmBalanceAction(address: string, options: OptionValues) {
    const balance = await getEVMBalanceOf(parseEVMAddressOrExit(address), getEvmUrl(options));

    logEVMBalance(balance, !options.json);
    process.exit(0);
}
