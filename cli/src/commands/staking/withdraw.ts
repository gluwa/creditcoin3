import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { initCallerKeyring } from '../../lib/account/keyring';
import { urlOption } from '../options';

export function makeWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded');
    cmd.description('Withdraw unbonded funds from a stash account');
    cmd.addOption(urlOption);
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initCallerKeyring(options);

    const address = keyring.address;
    const status = await getValidatorStatus(address, api);
    requireStatus(status, 'canWithdraw', 'Cannot perform action, there are no unlocked funds to withdraw');

    const slashingSpans = await api.query.staking.slashingSpans(keyring.address);
    const slashingSpansCount = slashingSpans.isSome ? slashingSpans.unwrap().lastNonzeroSlash : 0;
    const withdrawUnbondTx = api.tx.staking.withdrawUnbonded(slashingSpansCount);

    await requireEnoughFundsToSend(withdrawUnbondTx, keyring.address, api);

    const result = await signSendAndWatch(withdrawUnbondTx, api, keyring);

    console.log(result.info);
    process.exit(0);
}
