import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { getBalance, logBalance } from '../lib/balance';
import { parseAddressOrExit, parseBoolean, requiredInput } from '../lib/parsing';
import { getEvmUrl } from '../lib/evm/rpc';
import { getEVMBalanceOf } from '../lib/evm/balance';
import { substrateAddressToEvmAddress } from '../lib/evm/address';

export function makeBalanceCommand() {
    const cmd = new Command('balance');
    cmd.description('Get balance of an account');
    cmd.option('-a, --address [address]', 'Specify address to get balance of');
    cmd.option('--json', 'Output as JSON');
    cmd.action(balanceAction);
    return cmd;
}

async function balanceAction(options: OptionValues) {
    const json = parseBoolean(options.json);
    const { api } = await newApi(options.url as string);

    const address = parseAddressOrExit(
        requiredInput(options.address, 'Failed to show balance: Must specify an address'),
    );

    const balance = await getBalance(address, api);

    const evmAddress = substrateAddressToEvmAddress(address);
    const evmBalance = new BN((await getEVMBalanceOf(evmAddress, getEvmUrl(options))).toString());
    balance.evm = evmBalance;

    logBalance(balance, !json);

    process.exit(0);
}
