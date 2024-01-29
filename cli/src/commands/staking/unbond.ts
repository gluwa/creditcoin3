// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

import { Command, OptionValues } from 'commander';
import { newApi, BN } from '../../lib';
import { ApiPromise } from '@polkadot/api';
import { getBalance } from '../../lib/balance';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { getValidatorStatus, requireStatus } from '../../lib/staking';
import { initKeyring, validatorAddress } from '../../lib/account/keyring';
import { amountOption, parseSubstrateAddress } from '../options';

export function makeUnbondCommand() {
    const cmd = new Command('unbond');
    cmd.description('Schedule a bonded CTC to be unlocked');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.action(unbondAction);
    return cmd;
}

async function unbondAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const interactive = setInteractivity(options);

    const amount = options.amount as BN;

    // Build accounts
    const caller = await initKeyring(options);

    // We need to check the staking ledger of the caller even if we are using a proxy
    const validator_addr = validatorAddress(caller);
    const status = await getValidatorStatus(validator_addr, api);
    requireStatus(status, 'bonded');

    // // Check if amount specified exceeds total bonded funds
    await checkIfUnbodingMax(validator_addr, amount, api, interactive);

    // Unbond transaction
    const tx = api.tx.staking.unbond(amount.toString());

    await requireKeyringHasSufficientFunds(tx, caller, api);
    const result = await signSendAndWatchCcKeyring(tx, api, caller);
    console.log(result.info);
    process.exit(result.status);
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
