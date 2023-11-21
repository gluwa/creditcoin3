import { Command, OptionValues } from 'commander';
import { ApiPromise, BN, newApi } from '../../lib';
import { bond, parseRewardDestination } from '../../lib/staking';
import { promptContinue, setInteractivity } from '../../lib/interactive';
import { AccountBalance, getBalance, toCTCString, checkAmount } from '../../lib/balance';

import { inputOrDefault, parseAmountOrExit, parseBoolean, parseChoiceOrExit, requiredInput } from '../../lib/parsing';
import { initCallerKeyring } from '../../lib/account/keyring';

export function makeBondCommand() {
    const cmd = new Command('bond');
    cmd.description('Bond CTC in an account');
    cmd.option('-a, --amount [amount]', 'Amount to bond');
    cmd.option(
        '-r, --reward-destination [reward-destination]',
        'Specify reward destination account to use for new account',
    );
    cmd.option('-x, --extra', 'Bond as extra, adding more funds to an existing bond');
    cmd.action(bondAction);
    return cmd;
}

async function bondAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const { amount, rewardDestination, extra, interactive } = parseOptions(options);

    const callerKeyring = await initCallerKeyring(options);
    const callerAddress = callerKeyring.address;

    // Check if caller has enough balance
    await checkBalance(amount, api, callerAddress);

    console.log('Creating bond transaction...');
    console.log('Reward destination:', rewardDestination);
    console.log('Amount:', toCTCString(amount));
    if (extra) {
        console.log("Bonding as 'extra'; funds will be added to existing bond");
    }

    await promptContinue(interactive);

    const bondTxResult = await bond(callerKeyring, amount, rewardDestination, api, extra);

    console.log(bondTxResult.info);
    process.exit(0);
}

async function checkBalance(amount: BN, api: ApiPromise, address: string) {
    const balance = await getBalance(address, api);
    checkBalanceAgainstBondAmount(balance, amount);
}

function checkBalanceAgainstBondAmount(balance: AccountBalance, amount: BN) {
    if (balance.transferable.lt(amount)) {
        console.error(
            `Insufficient funds to bond ${toCTCString(amount)}, only ${toCTCString(balance.transferable)} available`,
        );
        process.exit(1);
    }
}

function parseOptions(options: OptionValues) {
    const amount = parseAmountOrExit(requiredInput(options.amount, 'Failed to bond: Must specify an amount to bond'));
    checkAmount(amount);

    const rewardDestination = parseRewardDestination(
        parseChoiceOrExit(inputOrDefault(options.rewardDestination, 'Staked'), ['Staked', 'caller']),
    );

    const extra = parseBoolean(options.extra);

    const interactive = setInteractivity(options);

    return { amount, rewardDestination, extra, interactive };
}
