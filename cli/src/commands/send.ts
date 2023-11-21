import { Command, OptionValues } from 'commander';
import { newApi } from '../lib';
import { requireEnoughFundsToSend, signSendAndWatch } from '../lib/tx';
import { parseAddressOrExit, parseAmountOrExit, requiredInput } from '../lib/parsing';
import { initCallerKeyring } from '../lib/account/keyring';

export function makeSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send CTC from an account');
    cmd.option('-a, --amount [amount]', 'Amount to send');
    cmd.option('-t, --to [to]', 'Specify recipient address');
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
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to send CTC: Must specify an amount'));

    const recipient = parseAddressOrExit(requiredInput(options.to, 'Failed to send CTC: Must specify a recipient'));

    return { amount, recipient };
}
