import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';

import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { JsonRpcProvider } from 'ethers';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { getEvmUrl } from '../../lib/evm/rpc';
import { urlOption } from '../options';

export function makeEvmWithdrawCommand() {
    const cmd = new Command('withdraw');
    cmd.description('Withdraw all funds from an associated EVM account to the owned Subtrate one');
    cmd.addOption(urlOption);
    cmd.action(evmWithdrawAction);
    return cmd;
}

async function evmWithdrawAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const caller = await initCallerKeyring(options);
    const evmAddress = substrateAddressToEvmAddress(caller.address);

    console.log(
        `Withdrawing all funds from associated EVM address ${evmAddress} into Substrate account ${caller.address}`,
    );

    const provider = new JsonRpcProvider(getEvmUrl(options));
    const balance = await provider.getBalance(evmAddress);
    console.log(balance);

    const tx = api.tx.evm.withdraw(evmAddress, balance.toString());

    await requireEnoughFundsToSend(tx, caller.address, api);
    const result = await signSendAndWatch(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}
