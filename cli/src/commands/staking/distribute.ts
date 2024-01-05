import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';
import { requiredInput, parseIntegerOrExit } from '../../lib/parsing';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { checkEraIsInHistory } from '../../lib/staking/era';
import { substrateAddressOption } from '../options';

export function makeDistributeRewardsCommand() {
    const cmd = new Command('distribute-rewards');
    cmd.description('Distribute all pending rewards for a particular validator');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option('-e, --era [era]', 'Specify era to distribute rewards for');
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
    const caller = await initCallerKeyring(options);

    const distributeTx = api.tx.staking.payoutStakers(validator, era);

    await requireEnoughFundsToSend(distributeTx, caller.address, api);

    const result = await signSendAndWatch(distributeTx, api, caller);

    console.log(result.info);
    process.exit(0);
}

function parseOptions(options: OptionValues) {
    const validator = options.substrateAddress as string;

    const era = parseIntegerOrExit(requiredInput(options.era, 'Failed to distribute rewards: Must specify an era'));

    if (era < 0) {
        console.error(`Failed to distribute rewards: Era ${era} is invalid; must be a positive integer`);
        process.exit(1);
    }

    return { validator, era };
}
