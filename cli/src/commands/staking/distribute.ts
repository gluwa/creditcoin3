import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initKeyring } from '../../lib/account/keyring';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { checkEraIsInHistory } from '../../lib/staking/era';
import { eraOption, substrateAddressOption, proxyForOption } from '../options';

export function makeDistributeRewardsCommand() {
    const cmd = new Command('distribute-rewards');
    cmd.description('Distribute all pending rewards for a particular validator');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.addOption(proxyForOption);
    cmd.addOption(eraOption.makeOptionMandatory());
    cmd.action(distributeRewardsAction);
    return cmd;
}

async function distributeRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const { validator, era } = parseOptions(options);

    const eraIsValid = await checkEraIsInHistory(era, api);
    if (!eraIsValid) {
        console.error(
            `Failed to distribute rewards: Era ${era} is not included in history; only the past 84 eras are eligible`,
        );
        process.exit(1);
    }

    // Any account can call the distribute_rewards extrinsic
    const caller = await initKeyring(options);

    const distributeTx = api.tx.staking.payoutStakers(validator, era);

    await requireKeyringHasSufficientFunds(distributeTx, caller, api);
    const result = await signSendAndWatchCcKeyring(distributeTx, api, caller);
    console.log(result.info);
    process.exit(result.status);
}

function parseOptions(options: OptionValues) {
    const validator = options.substrateAddress as string;
    const era = options.era as number;
    return { validator, era };
}
