import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, validatorAddress } from '../../lib/account/keyring';
import { useProxyOption } from '../options';

export function makeWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded');
    cmd.description('Withdraw unbonded funds from a stash account');
    cmd.addOption(useProxyOption);
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const validator_addr = validatorAddress(keyring);
    const status = await getValidatorStatus(validator_addr, api);
    requireStatus(status, 'canWithdraw', 'Cannot perform action, there are no unlocked funds to withdraw');

    const slashingSpans = await api.query.staking.slashingSpans(validator_addr);
    const slashingSpansCount = slashingSpans.isSome ? slashingSpans.unwrap().lastNonzeroSlash : 0;

    const withdrawUnbondTx = api.tx.staking.withdrawUnbonded(slashingSpansCount);

    await requireKeyringHasSufficientFunds(withdrawUnbondTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(withdrawUnbondTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
