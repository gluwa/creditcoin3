import { Command, OptionValues } from 'commander';
import { newApi } from '../lib';
import { getBalance, logBalance } from '../lib/balance';
import { parseAddressOrExit, parseBoolean, requiredInput } from '../lib/parsing';

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
    const { api } = await newApi(options.url);

    const address = parseAddressOrExit(
        requiredInput(options.address, 'Failed to show balance: Must specify an address'),
    );

    const balance = await getBalance(address, api);
    logBalance(balance, !json);

    process.exit(0);
}
