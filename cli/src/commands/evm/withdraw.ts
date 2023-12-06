import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';

import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { JsonRpcProvider } from 'ethers';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { getEvmUrl } from 'src/lib/evm/rpc';

export function makeEvmWithdrawCommand() {
    const cmd = new Command('withdraw');
    cmd.description('Withdraw all funds from an associated EVM account to the owned Subtrate one');
    cmd.option('--show-address', 'Show the associated EVM address and exit');
    cmd.action(evmWithdrawAction);
    return cmd;
}

async function evmWithdrawAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const caller = await initCallerKeyring(options);
    const evmAddress = substrateAddressToEvmAddress(caller.address);

    if (options.showAddress) {
        console.log(`Associated EVM address: ${evmAddress}. Send funds to it before trying to withdraw.`);
        process.exit(0);
    }

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
