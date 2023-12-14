import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { parseAmountOrExit, requiredInput } from '../../lib/parsing';
import { initCallerKeyring } from '../../lib/account/keyring';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';
import { toCTCString } from '../../lib/balance';
import { amountOption, evmAddressOption } from '../options';

export function makeEvmFundCommand() {
    const cmd = new Command('fund');
    cmd.description('Fund an EVM account from a Subtrate one');
    cmd.addOption(amountOption);
    cmd.addOption(evmAddressOption.makeOptionMandatory());
    cmd.action(evmFundAction);
    return cmd;
}

async function evmFundAction(options: OptionValues) {
    console.log(options);
    const { api } = await newApi(options.url as string);
    const { amount, recipient } = parseOptions(options);

    const evmAddress = recipient;
    const asociatedSubstrateAddress = evmAddressToSubstrateAddress(evmAddress);
    console.log(`Funding EVM address ${evmAddress} with ${toCTCString(amount)}`);
    console.log(`Seding to associated Substrate address ${asociatedSubstrateAddress}`);

    const caller = await initCallerKeyring(options);
    const tx = api.tx.balances.transfer(asociatedSubstrateAddress, amount.toString());
    await requireEnoughFundsToSend(tx, caller.address, api, amount);
    const result = await signSendAndWatch(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to send CTC: Must specify an amount'));

    const recipient = options.evmAddress as string;

    return { amount, recipient };
}
