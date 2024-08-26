import { Command, OptionValues } from 'commander';
import { getEvmUrl } from '../../lib/evm/rpc';
import { getEVMBalanceOf, logEVMBalance } from '../../lib/evm/balance';
import { evmAddressOption, jsonOption } from '../options';

export function makeEvmBalanceCommand() {
    const cmd = new Command('balance');
    cmd.description('Show balance of an EVM account');
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.addOption(jsonOption);
    cmd.action(evmBalanceAction);
    return cmd;
}

async function evmBalanceAction(options: OptionValues) {
    const balance = await getEVMBalanceOf(options.evmAddress as string, getEvmUrl(options));

    logEVMBalance(balance, !options.json);
    process.exit(0);
}
