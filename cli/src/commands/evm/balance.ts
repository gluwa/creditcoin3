import { Command, OptionValues } from 'commander';
import { parseEVMAddressOrExit } from '../../lib/parsing';
import { getEvmUrl } from '../../lib/evm/rpc';
import { toCTCString } from '../../lib/balance';
import { BN } from '../../lib';
import { getEVMBalanceOf } from '../../lib/evm/balance';

export function makeEvmBalanceCommand() {
    const cmd = new Command('balance');
    cmd.description('Show balance of an EVM accoun');
    cmd.argument('<address>', 'Address to check balance of');
    cmd.action(evmBalanceAction);
    return cmd;
}

async function evmBalanceAction(address: string, options: OptionValues) {
    const balance = await getEVMBalanceOf(
        parseEVMAddressOrExit(address), getEvmUrl(options)
    );
    const humanBalance = toCTCString(new BN(balance.toString()), 2);
    console.log('Account balance on EVM: ' + humanBalance.toString());
    process.exit(0);
}
