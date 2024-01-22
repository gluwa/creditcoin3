import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring, initProxyKeyring } from '../../lib/account/keyring';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { checkEraIsInHistory } from '../../lib/staking/era';
import { eraOption, parseSubstrateAddress, substrateAddressOption } from '../options';

export function makeDistributeRewardsCommand ()
{
    const cmd = new Command('distribute-rewards');
    cmd.description('Distribute all pending rewards for a particular validator');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.addOption(eraOption.makeOptionMandatory());
    cmd.action(distributeRewardsAction);
    return cmd;
}

async function distributeRewardsAction (options: OptionValues)
{
    const { api } = await newApi(options.url as string);

    const { validator, era } = parseOptions(options);

    const eraIsValid = await checkEraIsInHistory(era, api);
    if (!eraIsValid)
    {
        console.error(
            `Failed to distribute rewards: Era ${era} is not included in history; only the past 84 eras are eligible`,
        );
        process.exit(1);
    }

    // Any account can call the distribute_rewards extrinsic
    const caller = await initCallerKeyring(options);
    const proxy = await initProxyKeyring(options);

    let distributeTx = api.tx.staking.payoutStakers(validator, era);
    let callerAddress = caller?.address;
    let callerKeyring = caller;

    if (proxy && caller)
    {
        distributeTx = api.tx.proxy.proxy(options.address, null, distributeTx);
        callerAddress = proxy.address;
        callerKeyring = proxy;
    }

    if (!callerKeyring)
    {
        throw new Error('ERROR: keyring not initialized and proxy not selected');
    }
    if (!callerAddress)
    {
        throw new Error('ERROR: keyring not initialized and proxy not selected');
    }

    await requireEnoughFundsToSend(distributeTx, callerAddress, api);
    const result = await signSendAndWatch(distributeTx, api, callerKeyring);
    console.log(result.info);
    process.exit(0);
}

function parseOptions (options: OptionValues)
{
    const validator = options.substrateAddress as string;
    const era = options.era as number;
    return { validator, era };
}
