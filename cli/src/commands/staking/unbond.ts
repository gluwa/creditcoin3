// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

import { Command, OptionValues } from 'commander';
import { newApi, BN } from '../../lib';
import { ApiPromise } from '@polkadot/api';
import { getBalance } from '../../lib/balance';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { getValidatorStatus, requireStatus } from '../../lib/staking';
import { initCallerKeyring, initProxyKeyring } from '../../lib/account/keyring';
import { amountOption } from '../options';

export function makeUnbondCommand() {
    const cmd = new Command('unbond');
    cmd.description('Schedule a bonded CTC to be unlocked');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address of the proxied account (use only with -p, --proxy');
    cmd.action(unbondAction);
    return cmd;
}

async function unbondAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const interactive = setInteractivity(options);

    const amount = options.amount as BN;

    // Build account
    const caller = await initCallerKeyring(options);
    const proxy = await initProxyKeyring(options);

    // We need to check the staking ledger of the caller even if we are using a proxy
    const status = await getValidatorStatus(caller.address, api);
    requireStatus(status, 'bonded');

    // // Check if amount specified exceeds total bonded funds
    await checkIfUnbodingMax(caller.address, amount, api, interactive);

    // Unbond transaction
    let tx = api.tx.staking.unbond(amount.toString());
    if (options.proxy) {
        if (!proxy) {
            console.log('ERROR: proxy keyring not provided through $PROXY_SECRET or interactive prompt');
            process.exit(1);
        }
        if (!options.address) {
            console.log("ERROR: Address not supplied, provide with '--address <address>'");
            process.exit(1);
        }
        tx = api.tx.proxy.proxy(options.address, null, tx);
        await requireEnoughFundsToSend(tx, proxy.address, api, amount);
        const result = await signSendAndWatch(tx, api, proxy);
        console.log(result.info);
        process.exit(0);
    } else {
        await requireEnoughFundsToSend(tx, caller.address, api);
        const result = await signSendAndWatch(tx, api, caller);
        console.log(result.info);
        process.exit(0);
    }
}

async function checkIfUnbodingMax(
    address: string | undefined,
    unbondAmount: BN,
    api: ApiPromise,
    interactive: boolean,
) {
    if (!address) {
        console.error('ERROR: Unable to check if unbonding max. Address was undefined');
        process.exit(1);
    }
    const balance = await getBalance(address, api);
    if (balance.bonded.lt(unbondAmount)) {
        console.error('Warning: amount specified exceeds total bonded funds, will unbond all funds');
        await promptContinue(interactive);
    }
}
