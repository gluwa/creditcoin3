import * as timeDelta from 'time-delta';

import { ApiPromise } from '@polkadot/api';
import { BN } from '..';
import { readAmount, toCTCString } from '../balance';
import { getChainStatus } from '../chain/status';
import Table from 'cli-table3';

import type { DeriveSessionProgress, DeriveStakingAccount } from '@polkadot/api-derive/types';
import { BN_ONE, BN_ZERO } from '@polkadot/util';
import { U64 } from '@polkadot/types-codec';

interface Unlocking {
    remainingEras: BN;
    value: BN;
}

interface DeriveStakingAccountPartial {
    accountId: DeriveStakingAccount['accountId'] | string;
    unlocking?: Unlocking[];
}

function formatDaysHoursMinutes(ms: number) {
    // Note: argument here is milliseconds since the beginning of the epoch, 01.01.1970
    const asDate = new Date(ms);
    return timeDelta
        .create({
            locale: 'en',
        })
        .format(new Date(0), asDate);
}

// copied from https://github.com/polkadot-js/apps/blob/master/packages/react-components/src/StakingUnbonding.tsx#L33
function extractTotals(
    stakingInfo?: DeriveStakingAccountPartial,
    progress?: DeriveSessionProgress,
): [[Unlocking, BN, BN][], BN, boolean] {
    if (!stakingInfo?.unlocking || !progress) {
        return [[], BN_ZERO, false];
    }

    const isStalled = progress.eraProgress.gt(BN_ZERO) && progress.eraProgress.gt(progress.eraLength);
    const mapped = stakingInfo.unlocking
        .filter(({ remainingEras, value }) => value.gt(BN_ZERO) && remainingEras.gt(BN_ZERO))
        .map((unlock): [Unlocking, BN, BN] => [
            unlock,
            unlock.remainingEras,
            unlock.remainingEras
                .sub(BN_ONE)
                .imul(progress.eraLength)
                .iadd(progress.eraLength)
                .isub(
                    // in the case of a stalled era, this would not be accurate. We apply the mod here
                    // otherwise we would enter into negative values (which is "accurate" since we are
                    // overdue, but confusing since it implied it needed to be done already).
                    //
                    // This does mean that in cases of era stalls we would have an jiggling time, i.e.
                    // would be down and then when a session completes, would be higher again, just to
                    // repeat the cycle again
                    //
                    // See https://github.com/polkadot-js/apps/issues/9397#issuecomment-1532465939
                    isStalled ? progress.eraProgress.mod(progress.eraLength) : progress.eraProgress,
                ),
        ]);
    const total = mapped.reduce((total_, [{ value }]) => total_.iadd(value), new BN(0));

    return [mapped, total, isStalled];
}

export async function getValidatorStatus(stash: string | undefined, api: ApiPromise) {
    if (!stash) {
        return;
    }

    // Get the staking information for the stash
    const [res, progress] = await Promise.all([api.derive.staking.account(stash), api.derive.session.progress()]);

    const [mapped, _total, _isStalled] = extractTotals(res, progress);
    const expectedBlockTime = (api.consts.babe.expectedBlockTime as U64).toNumber();
    // see https://github.com/polkadot-js/apps/blob/master/packages/react-components/src/StakingUnbonding.tsx#L94
    const nextUnlocking = mapped.map(([{ value }, _eras, blocks], _index) => {
        return {
            amount: value,
            blocks,
            millis: blocks.toNumber() * expectedBlockTime,
        };
    });

    // Get the total staked amount
    const totalStaked = readAmount(res.stakingLedger.total.toString());
    const bonded = totalStaked.gt(new BN(0));
    const readyForWithdraw = res.redeemable ? res.redeemable : new BN(0);
    const canWithdraw = readyForWithdraw > new BN(0);

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
        nextUnlocking,
    };

    return validatorStatus;
}

export async function validatorStatusTable(status: Status | undefined, api: ApiPromise, humanReadable = true) {
    if (!status) {
        throw new Error('Status was undefined');
    }

    const chainStatus = await getChainStatus(api);

    const table = new Table({
        head: [
            `Active: ${chainStatus.eraInfo.activeEra}; Current: ${chainStatus.eraInfo.currentEra}; Session: ${chainStatus.eraInfo.currentSession}`,
            `Block: ${chainStatus.bestNumber}; Finalized: ${chainStatus.bestFinalizedNumber}`,
        ],
    });
    table.push(['Bonded', status.bonded ? 'Yes' : 'No']);
    table.push(['Validating', status.validating ? 'Yes' : 'No']);
    table.push(['Waiting', status.waiting ? 'Yes' : 'No']);
    table.push(['Active', status.active ? 'Yes' : 'No']);
    table.push(['Can withdraw', status.canWithdraw ? 'Yes' : 'No']);
    if (status.canWithdraw) {
        table.push(['Unlocked funds', toCTCString(status.readyForWithdraw)]);
    }

    if (!status.nextUnlocking.length) {
        table.push(['Next unlocking', 'None']);
    } else {
        status.nextUnlocking.forEach((chunk) => {
            const hrAmount = toCTCString(chunk.amount);
            let hrTime = chunk.millis;
            if (humanReadable) {
                hrTime = formatDaysHoursMinutes(hrTime);
            }

            table.push(['Next unlocking', `${hrAmount} in ${hrTime}; ${chunk.blocks.toString()} blocks`]);
        });
    }

    return table;
}

export async function printValidatorStatus(status: Status | undefined, api: ApiPromise) {
    const table = await validatorStatusTable(status, api);
    console.log(table.toString());
}

export function requireStatus(status: Status | undefined, condition: keyof Status, message?: string) {
    if (!status) {
        console.error('ERROR: Status was undefined');
        process.exit(1);
    }
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
    readyForWithdraw: Balance;
    nextUnlocking: UnlockChunkWithInfo[];
}

interface UnlockChunkWithInfo {
    amount: Balance;
    blocks: BN;
    millis: number;
}

type Balance = BN;
