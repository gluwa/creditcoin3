// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

import { Command, OptionValues } from 'commander';
import { newApi, BN } from '../../lib';
import { ApiPromise } from '@polkadot/api';
import { getBalance } from '../../lib/balance';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { getValidatorStatus, requireStatus } from '../../lib/staking';
import { initCallerKeyring } from '../../lib/account/keyring';
import { amountOption } from '../options';

export function makeUnbondCommand() {
    const cmd = new Command('unbond');
    cmd.description('Schedule a bonded CTC to be unlocked');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.action(unbondAction);
    return cmd;
}

async function unbondAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const interactive = setInteractivity(options);

    const amount = options.amount as BN;

    // Build account
    const caller = await initCallerKeyring(options);

    const status = await getValidatorStatus(caller.address, api);
    requireStatus(status, 'bonded');

    // // Check if amount specified exceeds total bonded funds
    await checkIfUnbodingMax(caller.address, amount, api, interactive);

    // Unbond transaction
    const tx = api.tx.staking.unbond(amount.toString());
    await requireEnoughFundsToSend(tx, caller.address, api);

    const result = await signSendAndWatch(tx, api, caller);

    console.log(result.info);
    process.exit(0);
}

async function checkIfUnbodingMax(address: string, unbondAmount: BN, api: ApiPromise, interactive: boolean) {
    const balance = await getBalance(address, api);
    if (balance.bonded.lt(unbondAmount)) {
        console.error('Warning: amount specified exceeds total bonded funds, will unbond all funds');
        await promptContinue(interactive);
    }
}
