import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from '../../lib/account/keyring';
import { parseAddressOrExit, requiredInput, parseIntegerOrExit } from '../../lib/parsing';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { checkEraIsInHistory } from '../../lib/staking/era';

export function makeDistributeRewardsCommand() {
    const cmd = new Command('distribute-rewards');
    cmd.description('Distribute all pending rewards for all validators');
    cmd.option('-v, --validator-id [address]', 'Specify the address of Validator to distribute rewards for');
    cmd.option('-e, --era [era]', 'Specify era to distribute rewards for');
    cmd.action(distributeRewardsAction);
    return cmd;
}

async function distributeRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url);

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
    const validator = parseAddressOrExit(
        requiredInput(options.validatorId, 'Failed to distribute rewards: Must specify a validator address'),
    );

    const era = parseIntegerOrExit(requiredInput(options.era, 'Failed to distribute rewards: Must specify an era'));

    if (era < 0) {
        console.error(`Failed to distribute rewards: Era ${era} is invalid; must be a positive integer`);
        process.exit(1);
    }

    return { validator, era };
}
