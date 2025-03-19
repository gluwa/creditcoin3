import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { substrateAddressOption } from '../options';
import { getBalancesAll, toCTCString } from '../../lib/balance';
import Table from 'cli-table3';

export function showAttestorBalanceActionCommand() {
    const cmd = new Command('show-stash-balance');
    cmd.description('Show balance of the stash account for attestors');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(showStashBalanceAction);
    return cmd;
}

async function showStashBalanceAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;

    const balanceAll = await getBalancesAll(address, api);

    const ledger = await api.query.attestation.ledger(address);

    if (ledger.isNone) {
        console.log(`No ledger found for ${address}`);
        process.exit(0);
    }

    let canWithdraw = 0;
    let unbondingAmount = 0;
    const ledgerValue = ledger.unwrap();
    const active = ledgerValue.active.unwrap();
    const totalStaked = ledgerValue.totalStaked.unwrap();

    const currentEra = await api.query.staking.currentEra();
    if (currentEra.isSome) {
        const currentEraValue = currentEra.unwrap();

        for (const unlocking of ledgerValue.unlocking) {
            if (unlocking.era.toNumber() <= currentEraValue.toNumber()) {
                canWithdraw += unlocking.value.toNumber();
            }
            unbondingAmount += unlocking.value.toNumber();
        }
    }

    const unclaimedRewardsStash = await api.query.attestation.accumulatedRewards(address);
    const unclaimedRewardsStashValue = unclaimedRewardsStash.unwrapOrDefault();

    const table = new Table({});

    table.push(
        ['Transferable', toCTCString(balanceAll.availableBalance, 4)],
        ['Locked', toCTCString(balanceAll.lockedBalance, 4)],
        ['Total', toCTCString(balanceAll.freeBalance.add(balanceAll.reservedBalance), 4)],
        ['TotalStake', toCTCString(totalStaked, 4)],
        ['ActiveStake', toCTCString(active, 4)],
        ['Unbonding', toCTCString(new BN(unbondingAmount), 4)],
        ['CanWithdraw', toCTCString(new BN(canWithdraw), 4)],
        ['UnclaimedRewards', toCTCString(new BN(unclaimedRewardsStashValue), 4)],
    );

    console.log(`Address: ${address}`);
    console.log(table.toString());

    process.exit(0);
}
