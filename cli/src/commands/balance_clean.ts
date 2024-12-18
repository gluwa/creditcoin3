// import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { getBalance, logBalance, toCTCString } from '../lib/balance';
import { parseBoolean } from '../lib/parsing';
import { getEvmUrl } from '../lib/evm/rpc';
import { getEVMBalanceOf } from '../lib/evm/balance';
import { substrateAddressToEvmAddress } from '../lib/evm/address';
import { substrateAddressOption, jsonOption } from './options';

// export function makeBalanceCommand() {
//     const cmd = new Command('balance');
//     cmd.description('Get balance of an account');
//     cmd.addOption(substrateAddressOption.makeOptionMandatory());
//     cmd.addOption(jsonOption);
//     cmd.action(balanceAction);
//     return cmd;
// }

export type OptionValues = Record<string, any>;

export async function balanceAction(options: OptionValues) {
    const json = parseBoolean(options.json);
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;

    const balance = await getBalance(address, api);
    console.log('balance:', balance);

    const evmAddress = substrateAddressToEvmAddress(address);
    console.log('balance:', evmAddress);
    const evmBalance = new BN((await getEVMBalanceOf(evmAddress, getEvmUrl(options))).ctc.toString());
    balance.evm = evmBalance;
    balance.total = balance.total.add(evmBalance);

    console.log('balance:', balance);

    console.log(toCTCString(balance.transferable, 4));

    // logBalance(balance, !json);

    // process.exit(0);
}
