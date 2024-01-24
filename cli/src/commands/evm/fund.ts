import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { requireEnoughFundsToSend, signSendAndWatch, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initCallerKeyring, initKeyring } from '../../lib/account/keyring';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';
import { toCTCString } from '../../lib/balance';
import { amountOption, evmAddressOption, parseSubstrateAddress } from '../options';

export function makeEvmFundCommand() {
    const cmd = new Command('fund');
    cmd.description('Fund an EVM account from a Subtrate one');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [proxy addr]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.action(evmFundAction);
    return cmd;
}

async function evmFundAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);
    const { amount, recipient } = parseOptions(options);

    const evmAddress = recipient;
    const asociatedSubstrateAddress = evmAddressToSubstrateAddress(evmAddress);
    console.log(`Funding EVM address ${evmAddress} with ${toCTCString(amount)}`);
    console.log(`Sending to associated Substrate address ${asociatedSubstrateAddress}`);

    const caller = await initKeyring(options);

    if (!caller) {
        throw new Error('Keyring not initialized and not using a proxy');
    }

    const tx = api.tx.balances.transfer(asociatedSubstrateAddress, amount.toString());
    await requireEnoughFundsToSend(tx,caller.pair.address, api, amount);
    const result = await signSendAndWatchCcKeyring(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = options.amount as BN;

    const recipient = options.evmAddress as string;

    return { amount, recipient };
}
