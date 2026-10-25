import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { bond, parseRewardDestination } from '../../lib/staking';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { toCTCString, checkAmount } from '../../lib/balance';

import { inputOrDefault, parseBoolean, parseChoiceOrExit } from '../../lib/parsing';
import { initKeyring } from '../../lib/account/keyring';
import { amountOption, proxyForOption } from '../options';

export function makeBondCommand() {
    const cmd = new Command('bond');
    cmd.description('Bond CTC in an account');
    cmd.addOption(amountOption.makeOptionMandatory());
    cmd.option(
        '-r, --reward-destination [reward-destination]',
        'Specify reward destination account to use for new account',
    );
    cmd.option('-x, --extra', 'Bond as extra, adding more funds to an existing bond');
    cmd.addOption(proxyForOption);
    cmd.action(bondAction);
    return cmd;
}

async function bondAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const { amount, rewardDestination, extra, interactive } = parseOptions(options);

    const callerKeyring = await initKeyring(options);

    console.log('Creating bond transaction...');
    console.log('Reward destination:', rewardDestination);
    console.log('Amount:', toCTCString(amount));
    if (extra) {
        console.log("Bonding as 'extra'; funds will be added to existing bond");
    }

    await promptContinue(interactive);

    const bondTxResult = await bond(callerKeyring, amount, rewardDestination, api, extra);

    console.log(bondTxResult.info);
    process.exit(bondTxResult.status);
}

function parseOptions(options: OptionValues) {
    const amount = options.amount as BN;
    checkAmount(amount);

    const rewardDestination = parseRewardDestination(
        parseChoiceOrExit(inputOrDefault(options.rewardDestination, 'Staked'), ['Staked', 'Stash']),
    );

    const extra = parseBoolean(options.extra);
    const interactive = setInteractivity(options);

    return { amount, rewardDestination, extra, interactive };
}
