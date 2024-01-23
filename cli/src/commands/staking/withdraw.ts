import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireEnoughFundsToSend, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { parseSubstrateAddress } from '../options';

export function makeWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded');
    cmd.description('Withdraw unbonded funds from a stash account');
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [proxy addr]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const status = await getValidatorStatus(keyring.pair.address, api);
    requireStatus(status, 'canWithdraw', 'Cannot perform action, there are no unlocked funds to withdraw');

    const slashingSpans = await api.query.staking.slashingSpans(keyring.pair.address);
    const slashingSpansCount = slashingSpans.isSome ? slashingSpans.unwrap().lastNonzeroSlash : 0;

    const withdrawUnbondTx = api.tx.staking.withdrawUnbonded(slashingSpansCount);

    await requireEnoughFundsToSend(withdrawUnbondTx, keyring.pair.address, api);
    const result = await signSendAndWatchCcKeyring(withdrawUnbondTx, api, keyring);
    console.log(result.info);
    process.exit(0);
}
