import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { ApiPromise, BN, MICROUNITS_PER_CTC } from '..';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../tx';
import { CcKeyring } from '../account/keyring';

export type RewardDestination = 'Staked' | 'Stash';

export async function bond(
    stashKeyring: CcKeyring,
    amount: BN,
    rewardDestination: RewardDestination,
    api: ApiPromise,
    extra = false,
) {
    console.log(`Amount: ${amount.toString()}`);

    if (amount.lt(MICROUNITS_PER_CTC)) {
        throw new Error('Amount to bond must be at least 1');
    }

    let bondTx: SubmittableExtrinsic<'promise', ISubmittableResult>;

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amount.toString());
    } else {
        await hasBondedEnough(amount, api);

        bondTx = api.tx.staking.bond(amount.toString(), rewardDestination);
    }

    await requireKeyringHasSufficientFunds(bondTx, stashKeyring, api, amount);
    return await signSendAndWatchCcKeyring(bondTx, api, stashKeyring);
}

export async function hasBondedEnough(amount: BN, api: ApiPromise) {
    // Get min bond amount
    const minValidatorBond = await api.query.staking.minValidatorBond();

    // Should atleast bond the min validator bond amount on initial bond
    if (amount < minValidatorBond) {
        const amountMsg = new BN(minValidatorBond.toString()).div(MICROUNITS_PER_CTC);
        throw new Error(`Amount to bond must be at least: ${amountMsg.toString()} CTC (min validator bond amount)`);
    }
}

export function parseRewardDestination(rewardDestinationRaw: string): RewardDestination {
    // Capitalize first letter and lowercase the rest
    const rewardDestination =
        rewardDestinationRaw.charAt(0).toUpperCase() + rewardDestinationRaw.slice(1).toLowerCase();

    if (rewardDestination !== 'Staked' && rewardDestination !== 'Stash') {
        throw new Error("Invalid reward destination, must be one of 'Staked' or 'Stash'");
    } else {
        return rewardDestination;
    }
}

export async function setStakingConfig(
    callerKeyring: CcKeyring,
    api: ApiPromise,
    minNomitatorBond: any,
    minValidatorBond: any,
    maxNominatorCount: any,
    maxValidatorCount: any,
    chillThreshold: any,
    minCommission: any,
) {
    const configTx = api.tx.staking.setStakingConfigs(
        setStakingConfigOp(minNomitatorBond),
        setStakingConfigOp(minValidatorBond),
        setStakingConfigOp(maxNominatorCount),
        setStakingConfigOp(maxValidatorCount),
        setStakingConfigOp(chillThreshold),
        setStakingConfigOp(minCommission),
    );

    const sudoTx = api.tx.sudo.sudo(configTx);
    await signSendAndWatchCcKeyring(sudoTx, api, callerKeyring);
}

function setStakingConfigOp(op: any): any {
    if (op === 0) {
        op = { remove: null };
    } else if (op === null) {
        op = { noop: null };
    } else {
        op = { set: op };
    }

    return op;
}
