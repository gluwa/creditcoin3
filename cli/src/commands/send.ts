import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { requireEnoughFundsToSend, signSendAndWatch } from '../lib/tx';
import { initCallerKeyring } from '../lib/account/keyring';
import { amountOption, substrateAddressOption } from './options';

export function makeSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send CTC from an account');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(sendAction);
    return cmd;
}

async function sendAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const { amount, recipient } = parseOptions(options);

    const caller = await initCallerKeyring(options);

    const tx = api.tx.balances.transfer(recipient, amount.toString());

    await requireEnoughFundsToSend(tx, caller.address, api, amount);

    const result = await signSendAndWatch(tx, api, caller);
    console.log(result.info);

    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = options.amount as BN;

    const recipient = options.substrateAddress as string;

    return { amount, recipient };
}
