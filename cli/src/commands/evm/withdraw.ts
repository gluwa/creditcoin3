import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initKeyring } from '../../lib/account/keyring';

import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { JsonRpcProvider } from 'ethers';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { getEvmUrl } from '../../lib/evm/rpc';

export function makeEvmWithdrawCommand() {
    const cmd = new Command('withdraw');
    cmd.description('Withdraw all funds from an associated EVM account to the owned Subtrate one');
    cmd.action(evmWithdrawAction);
    return cmd;
}

async function evmWithdrawAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const caller = await initKeyring(options);

    const evmAddress = substrateAddressToEvmAddress(caller.pair.address);

    console.log(
        `Withdrawing all funds from associated EVM address ${evmAddress} into Substrate account ${caller.pair.address}`,
    );

    const provider = new JsonRpcProvider(getEvmUrl(options));
    const balance = await provider.getBalance(evmAddress);
    console.log(balance);

    const tx = api.tx.evm.withdraw(evmAddress, balance.toString());

    await requireKeyringHasSufficientFunds(tx, caller, api);
    const result = await signSendAndWatchCcKeyring(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}
