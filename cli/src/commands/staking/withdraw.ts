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

    const validator_addr = keyring.type === 'proxy' ? keyring.proxiedAddress : keyring.pair.address;
    const status = await getValidatorStatus(validator_addr, api);
    requireStatus(status, 'canWithdraw', 'Cannot perform action, there are no unlocked funds to withdraw');

    const slashingSpans = await api.query.staking.slashingSpans(validator_addr);
    const slashingSpansCount = slashingSpans.isSome ? slashingSpans.unwrap().lastNonzeroSlash : 0;

    const withdrawUnbondTx = api.tx.staking.withdrawUnbonded(slashingSpansCount);

    await requireEnoughFundsToSend(withdrawUnbondTx, validator_addr, api);
    const result = await signSendAndWatchCcKeyring(withdrawUnbondTx, api, keyring);
    console.log(result.info);
    process.exit(0);
}
