import { Command, OptionValues } from 'commander';
import { parseAmountOrExit, parseEVMAddressOrExit, requiredInput } from '../../lib/parsing';
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
    const balance = await getEVMBalanceOf(address, getEvmUrl(options));
    const humanBalance = toCTCString(new BN(balance.toString()), 2);
    console.log('Account balance on EVM: ' + humanBalance.toString());
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to send CTC: Must specify an amount'));
    const recipient = parseEVMAddressOrExit(requiredInput(options.to, 'Failed to send CTC: Must specify a recipient'));
    return { amount, recipient };
}
