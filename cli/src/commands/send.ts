import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../lib/tx';
import { initKeyring } from '../lib/account/keyring';
import { amountOption, substrateAddressOption, proxyForOption } from './options';
import { filterProxiesByAddress, hasProxyType, proxiesForAddress } from '../lib/proxy';

export function makeSendCommand() {
    const cmd = new Command('send');
    cmd.description('Send CTC from an account');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.addOption(proxyForOption);
    cmd.action(sendAction);
    return cmd;
}

async function sendAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);
    const { amount, recipient } = parseOptions(options);
    const caller = await initKeyring(options);

    if (caller.type === 'proxy') {
        const existingProxies = filterProxiesByAddress(
            caller.pair.address,
            await proxiesForAddress(caller.proxiedAddress, api),
        );

        if (existingProxies.length === 0) {
            console.log(`ERROR: ${caller.pair.address} is not a proxy for ${caller.proxiedAddress}`);
            process.exit(1);
        }

        if (!hasProxyType(existingProxies, 'All')) {
            console.log(
                `ERROR: The proxy ${caller.pair.address} for address ${caller.proxiedAddress} does not have permission to call extrinsics from the balances pallet`,
            );
            process.exit(1);
        }
    }

    const tx = api.tx.balances.transfer(recipient, amount.toString());
    await requireKeyringHasSufficientFunds(tx, caller, api, amount);
    const result = await signSendAndWatchCcKeyring(tx, api, caller);
    console.log(result.info);
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const amount = options.amount as BN;

    const recipient = options.substrateAddress as string;

    return { amount, recipient };
}
