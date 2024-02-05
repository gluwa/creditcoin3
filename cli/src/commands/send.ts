import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { requireEnoughFundsToSend, signSendAndWatchCcKeyring } from '../lib/tx';
import { initKeyring } from '../lib/account/keyring';
import { amountOption, parseSubstrateAddress, substrateAddressOption } from './options';

export function makeSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send CTC from an account');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [proxy addr]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.action(sendAction);
    return cmd;
}

async function sendAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const { amount, recipient } = parseOptions(options);

    const caller = await initKeyring(options);

    if (!caller) {
        throw new Error('Keyring not initialized and not using a proxy');
    }

    if (caller.type === 'proxy') {
        const [delegates, _] = await api.query.proxy.proxies(caller.proxiedAddress);

        if (delegates.toArray().find((delegate) => delegate.proxyType.toString() === 'All') === undefined) {
            console.log(
                `ERROR: The proxy ${caller.pair.address} for address ${caller.proxiedAddress} does not have permission to call extrinsics from the balances pallet`,
            );
            process.exit(1);
        }
    }

    const tx = api.tx.balances.transfer(recipient, amount.toString());
    const funderAddr = caller.type === 'proxy' ? caller.proxiedAddress : caller.pair.address;
    await requireEnoughFundsToSend(tx, funderAddr, api, amount);
    const result = await signSendAndWatchCcKeyring(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = options.amount as BN;

    const recipient = options.substrateAddress as string;

    return { amount, recipient };
}
