import { ApiPromise, parseUnits, MICROUNITS_PER_CTC } from '..';
import { BN } from '@polkadot/util';
import Table from 'cli-table3';

import type { DeriveBalancesAll, DeriveStakingAccount } from '@polkadot/api-derive/types';

export function parseCTCString(amount: string): BN {
    try {
        const parsed = positiveBigNumberFromString(amount);
        return new BN(parsed.toString());
    } catch (_e) {
        console.error(`Unable to parse CTC amount: ${amount}`);
        process.exit(1);
    }
}

export function toCTCString(amount: BN, decimals = 18): string {
    const amountStrLen = amount.toString().length;
    if (amountStrLen < 18 - decimals) {
        decimals = 18;
    }
    const CTC = amount.div(MICROUNITS_PER_CTC);
    const remainder = amount.mod(MICROUNITS_PER_CTC);
    const remainderString = remainder.toString().padStart(18, '0').slice(0, decimals);
    return `${CTC.toString()}.${remainderString} CTC`;
}

export function readAmount(amount: string): BN {
    return new BN(amount);
}

export interface AccountBalance {
    address: string;
    transferable: BN;
    locked: BN;
    bonded: BN;
    evm: BN;
    total: BN;
    unbonding: BN;
}

export async function getBalance(address: string, api: ApiPromise) {
    const balacesAll = await getBalancesAll(address, api);
    const stakingInfo = await getStakingInfo(address, api);
    const stakingHold = await getStakingHoldBalance(address, api);

    const total = balacesAll.freeBalance.add(balacesAll.reservedBalance);
    const transferable = getTransferable(balacesAll);

    const balance: AccountBalance = {
        address,
        transferable,
        bonded: stakingInfo?.stakingLedger.active?.unwrap() || new BN(0),
        evm: new BN(0), // Get Balance does not reflect EVM balance, it must be added manually
        // Staking-scoped encumbrance: the legacy lock plus the HoldReason::Staking hold that
        // replaced it. Deliberately NOT `total - transferable`: that would also sweep in
        // unrelated reserves such as the proxy deposit, which is not locked stake.
        locked: balacesAll.lockedBalance.add(stakingHold),
        total,
        unbonding: calcUnbonding(stakingInfo),
    };

    return balance;
}

/**
 * Spendable balance, per the runtime's own rule.
 *
 * `availableBalance` is `max(0, free - lockedBalance)`, which only looks at `balances.locks`.
 * That is wrong on two counts since pallet-staking moved bonded funds to a hold: it misses held
 * stake entirely, and because `frozen` is a floor on `free + reserved` (so anything reserved
 * discounts it) it also subtracts freezes that a reserve already covers. `transferable`
 * implements `free - max(maybeEd, frozen - reserved)`, matching pallet-balances
 * `reducible_balance()`. polkadot-js documents `availableBalance` as legacy and points here.
 *
 * It is `null` on chains still returning the pre-FrameSystemAccountInfo shape, hence the
 * fallback, and @polkadot/api 15.x omits the outer clamp, hence the BN.max.
 */
export function getTransferable(balancesAll: DeriveBalancesAll): BN {
    const transferable = balancesAll.transferable ?? balancesAll.availableBalance;
    return BN.max(new BN(0), transferable);
}

/**
 * Bonded stake now sits in a HoldReason::Staking hold rather than a `balances.locks` lock, so
 * `lockedBalance` alone reports zero for any stash that has been through the lock-to-hold
 * migration.
 */
async function getStakingHoldBalance(address: string, api: ApiPromise): Promise<BN> {
    const holds = await api.query.balances.holds(address);
    return holds.filter((hold) => hold.id.isStaking).reduce((total, hold) => total.iadd(hold.amount), new BN(0));
}

export async function getBalancesAll(address: string, api: ApiPromise) {
    const balance = await api.derive.balances.all(address);
    return balance;
}

async function getStakingInfo(address: string, api: ApiPromise) {
    const stakingInfo = await api.derive.staking.account(address);
    return stakingInfo;
}

function calcUnbonding(stakingInfo?: DeriveStakingAccount) {
    if (!stakingInfo?.unlocking) {
        return new BN(0);
    }

    const filtered = stakingInfo.unlocking
        .filter(({ remainingEras, value }) => value.gt(new BN(0)) && remainingEras.gt(new BN(0)))
        .map((unlock) => unlock.value);
    const unbonding = filtered.reduce((total, value) => total.iadd(value), new BN(0));

    return unbonding;
}

export function logBalance(balance: AccountBalance, human = true) {
    if (human) {
        printBalance(balance);
    } else {
        printJsonBalance(balance);
    }
}

export function printBalance(balance: AccountBalance) {
    const table = new Table({});

    table.push(
        ['Transferable', toCTCString(balance.transferable, 4)],
        ['Locked', toCTCString(balance.locked, 4)],
        ['Bonded', toCTCString(balance.bonded, 4)],
        ['EVM', toCTCString(balance.evm, 4)],
        ['Unbonding', toCTCString(balance.unbonding, 4)],
        ['Total', toCTCString(balance.total, 4)],
    );

    console.log(`Address: ${balance.address}`);
    console.log(table.toString());
}

export function printJsonBalance(balance: AccountBalance) {
    const jsonBalance = {
        balance: {
            address: balance.address,
            transferable: balance.transferable.toString(),
            bonded: balance.bonded.toString(),
            evm: balance.evm.toString(),
            locked: balance.locked.toString(),
            unbonding: balance.unbonding.toString(),
            total: balance.total.toString(),
        },
    };
    console.log(JSON.stringify(jsonBalance, null, 2));
}

export function checkAmount(amount: BN) {
    if (!amount) {
        console.log('Must specify amount to bond');
        process.exit(1);
    } else if (amount.lt(MICROUNITS_PER_CTC)) {
        console.log('Bond amount must be at least 1 CTC');
        process.exit(1);
    }
    return amount;
}

function positiveBigNumberFromString(amount: string) {
    const parsedValue = parseUnits(amount, 18);

    if (parsedValue === BigInt(0)) {
        console.error('Failed to parse amount, must be greater than 0');
        process.exit(1);
    }

    if (parsedValue < BigInt(0)) {
        console.error('Failed to parse amount, must be a positive number');
        process.exit(1);
    }

    return parsedValue;
}
