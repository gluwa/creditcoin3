// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

import { Command, OptionValues } from 'commander';
import { newApi } from '../../api';
import { ApiPromise } from '@polkadot/api';
import { BN } from '../../lib';
import { getBalance } from '../../lib/balance';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { parseAmountOrExit, requiredInput } from '../../lib/parsing';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { initStashKeyring } from '../../lib/account/keyring';
import {
    getValidatorStatus,
    requireStatus,
} from 'src/lib/staking/validatorStatus';

export function makeUnbondCommand() {
    const cmd = new Command('unbond');
    cmd.description('Schedule a portion of the stash to be unlocked');
    cmd.option('-a, --amount [amount]', 'Amount to send');
    cmd.action(unbondAction);
    return cmd;
}

async function unbondAction(options: OptionValues) {
    const { api } = await newApi(options.url);

    const interactive = setInteractivity(options);

    const amount = parseAmountOrExit(
        requiredInput(
            options.amount,
            'Failed to unbond: Must specify an amount'
        )
    );

    // Build account
    const stash = await initStashKeyring(options);

    const status = await getValidatorStatus(stash.address, api);
    requireStatus(status, 'bonded');

    // // Check if amount specified exceeds total bonded funds
    await checkIfUnbodingMax(stash.address, amount, api, interactive);

    // Unbond transaction
    const tx = api.tx.staking.unbond(amount.toString());
    await requireEnoughFundsToSend(tx, stash.address, api);

    const result = await signSendAndWatch(tx, api, stash);

    console.log(result.info);
    process.exit(0);
}

async function checkIfUnbodingMax(
    address: string,
    unbondAmount: BN,
    api: ApiPromise,
    interactive: boolean
) {
    const balance = await getBalance(address, api);
    if (balance.bonded.lt(unbondAmount)) {
        console.error(
            'Warning: amount specified exceeds total bonded funds, will unbond all funds'
        );
        await promptContinue(interactive);
    }
}
