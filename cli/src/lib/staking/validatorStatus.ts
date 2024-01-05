import { ApiPromise } from '@polkadot/api';
import { BN } from '..';
import { readAmount, readAmountFromHex, toCTCString } from '../balance';
import { timeTillEra } from './era';
import Table from 'cli-table3';
import { PalletStakingUnlockChunk } from '@polkadot/types/lookup';

function formatDaysHoursMinutes(ms: number) {
    const days = Math.floor(ms / (24 * 60 * 60 * 1000));
    const daysms = ms % (24 * 60 * 60 * 1000);
    const hours = Math.floor(daysms / (60 * 60 * 1000));
    const hoursms = ms % (60 * 60 * 1000);
    const minutes = Math.floor(hoursms / (60 * 1000));
    const minutesms = ms % (60 * 1000);
    const sec = Math.floor(minutesms / 1000);

    const daysString = days > 0 ? `${days} days, ` : ``;
    const hoursString = hours > 0 ? `${hours} hours, ` : ``;
    const minutesString = minutes > 0 ? `${minutes} minutes, ` : ``;
    const secString = sec > 0 ? `${sec} seconds` : ``;

    return `${daysString}${hoursString}${minutesString}${secString}`;
}

export async function getValidatorStatus(address: string, api: ApiPromise) {
    const stash = address;

    // Get the staking information for the stash
    const res = await api.derive.staking.account(stash);

    // Get the total staked amount
    const totalStaked = readAmount(res.stakingLedger.total.toString());
    const bonded = totalStaked.gt(new BN(0));

    // Get information about any unbonding tokens and unlocked chunks
    const unlockingRes = res.stakingLedger.unlocking;
    const currentEra = (await api.query.staking.currentEra()).unwrap();
    const unlocking = unlockingRes
        ? unlockingRes.filter((u: PalletStakingUnlockChunk) => u.era.toNumber() > currentEra.toNumber())
        : [];

    const redeemable = res.redeemable ? readAmountFromHex(res.redeemable.toString()) : new BN(0);

    // Get the unlocked chunks that are ready for withdrawal
    // by comparing the era of each chunk to the current era
    const readyForWithdraw = res.stakingLedger.unlocking
        .map((u: PalletStakingUnlockChunk) => {
            const chunk: UnlockChunk = {
                era: u.era.toNumber(),
                value: u.value.toBn(),
            };
            return chunk;
        })
        .filter((u: UnlockChunk) => u.era < currentEra.toNumber());

    const canWithdraw = readyForWithdraw.length > 0;

    const nextUnbondingDate = unlocking.length > 0 ? unlocking[0].era.toNumber() : null;

    const nextUnbondingAmount = unlocking.length > 0 ? unlocking[0].value.toBn() : null;

    // Get lists of all validators, active validators, and waiting validators
    const validatorEntries = await api.query.staking.validators
        .entries()
        .then((r) => r.map((v) => v[0].toHuman()?.toString()));
    const activeValidatorsRes = await api.derive.staking.validators();
    const activeValidators: string[] = activeValidatorsRes.validators.map((v) => v.toString());
    const waitingValidators = validatorEntries.filter((v) => {
        if (v !== undefined) {
            return !activeValidators.includes(v);
        } else {
            return false;
        }
    });

    // Check if the validator is validating, waiting, or active
    const validating = validatorEntries.includes(stash);
    const waiting = waitingValidators.includes(stash);
    const active = activeValidators.includes(stash);

    const validatorStatus: Status = {
        bonded,
        stash,
        validating,
        waiting,
        active,
        canWithdraw,
        readyForWithdraw,
        nextUnbondingDate,
        nextUnbondingAmount: nextUnbondingAmount ?? new BN(0),
        redeemable,
    };

    return validatorStatus;
}

export async function printValidatorStatus(status: Status, api: ApiPromise) {
    const table = new Table({
        head: ['Status'],
    });
    table.push(['Bonded', status.bonded ? 'Yes' : 'No']);
    table.push(['Validating', status.validating ? 'Yes' : 'No']);
    table.push(['Waiting', status.waiting ? 'Yes' : 'No']);
    table.push(['Active', status.active ? 'Yes' : 'No']);
    table.push(['Can withdraw', status.canWithdraw ? 'Yes' : 'No']);
    if (status.canWithdraw) {
        status.readyForWithdraw.forEach((chunk) => {
            table.push([`Unlocked since era ${chunk.era}`, toCTCString(chunk.value)]);
        });
    }
    let nextUnlocking;
    if (status.nextUnbondingAmount?.eq(new BN(0))) {
        nextUnlocking = 'None';
    } else if (status.nextUnbondingAmount && status.nextUnbondingDate) {
        const nextUnbondingAmount = toCTCString(status.nextUnbondingAmount);
        const nextUnbondingDate = await timeTillEra(api, status.nextUnbondingDate);
        nextUnlocking = `${nextUnbondingAmount} in ${formatDaysHoursMinutes(nextUnbondingDate.toNumber())}`;
    }
    table.push(['Next unlocking', nextUnlocking]);

    console.log(table.toString());
}

export function requireStatus(status: Status, condition: keyof Status, message?: string) {
    if (!status[condition]) {
        console.error(message ?? `Cannot perform action, validator is not ${condition.toString()}`);
        process.exit(1);
    }
}

export interface Status {
    bonded: boolean;
    stash?: string;
    validating: boolean;
    waiting: boolean;
    active: boolean;
    canWithdraw: boolean;
    readyForWithdraw: UnlockChunk[];
    nextUnbondingDate: Option<number>;
    nextUnbondingAmount: Option<Balance>;
    redeemable: Balance;
}

interface UnlockChunk {
    era: number;
    value: Balance;
}

type Balance = BN;

type Option<T> = T | null;
