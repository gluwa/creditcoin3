import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { substrateAddressOption } from '../options';
import { getBalancesAll, toCTCString } from '../../lib/balance';
import Table from 'cli-table3';

export function showAttestorBalanceActionCommand() {
    const cmd = new Command('show-attestor-balance');
    cmd.description('Show balance of an attestor');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.action(showAttestorBalanceAction);
    return cmd;
}

async function showAttestorBalanceAction(options: OptionValues) {
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
    let active = ledgerValue.active.unwrap();
    let totalStaked = ledgerValue.totalStaked.unwrap();

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

    let unclaimedRewardsAttestorValue = 0;
    const unclaimedRewardsAttestor = await api.query.attestation.accumulatedRewards(address);
    if (unclaimedRewardsAttestor.isSome) {
        unclaimedRewardsAttestorValue = unclaimedRewardsAttestor.unwrap().toNumber();
    }

    const table = new Table({});

    table.push(
        ['Transferable', toCTCString(balanceAll.availableBalance, 4)],
        ['Locked', toCTCString(balanceAll.lockedBalance, 4)],
        ['Total', toCTCString(balanceAll.freeBalance.add(balanceAll.reservedBalance), 4)],
        ['AttestorTotalStake', toCTCString(totalStaked, 4)],
        ['AttestorActiveStake', toCTCString(active, 4)],
        ['Unbonding', toCTCString(new BN(unbondingAmount), 4)],
        ['CanWithdraw', toCTCString(new BN(canWithdraw), 4)],
        ['UnclaimedRewards', toCTCString(new BN(unclaimedRewardsAttestorValue), 4)],
    );

    console.log(`Address: ${address}`);
    console.log(table.toString());

    process.exit(0);
}
