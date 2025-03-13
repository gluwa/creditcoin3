import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { substrateAddressOption } from '../options';

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

    console.log(`Unclaimed rewards for : ${address} is ${unclaimedRewardsAttestor.toString()}`);
    process.exit(0);
}
