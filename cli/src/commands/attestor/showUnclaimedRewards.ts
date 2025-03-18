import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { substrateAddressOption } from '../options';
import { toCTCString } from '../../lib/balance';

export function showClaimRewardsCommand() {
    const cmd = new Command('show-unclaimed-rewards');
    cmd.description('Show unclaimed rewards that attestor has earned');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(showUnclaimedRewardsAction);
    return cmd;
}

async function showUnclaimedRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;

    const unclaimedRewardsAttestor = await api.query.attestation.accumulatedRewards(address);

    if (unclaimedRewardsAttestor.isNone) {
        console.log(`No rewards to claim for address ${address}`);
        process.exit(0);
    }

    console.log(`Unclaimed rewards for : ${address} is ${toCTCString(new BN(unclaimedRewardsAttestor.unwrap()), 4)}`);
    process.exit(0);
}
