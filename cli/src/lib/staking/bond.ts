import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { ApiPromise, BN, KeyringPair, MICROUNITS_PER_CTC } from '..';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring, signSendAndWatch } from '../tx';
import { CcKeyring } from '../account/keyring';
import { getBalance } from '../balance';

export type RewardDestination = 'Staked' | 'Stash';

export async function bond(
    stashKeyring: CcKeyring,
    amount: BN,
    rewardDestination: RewardDestination,
    api: ApiPromise,
    extra = false,
) {
    console.log(`Amount: ${amount.toString()}`);

    const precision = BigInt(MICROUNITS_PER_CTC);

    if (BigInt(amount.toString()) < precision) {
        throw new Error('Amount to bond must be at least 1');
    }

    const amountInMicroUnits = amount;

    let bondTx: SubmittableExtrinsic<'promise', ISubmittableResult>;

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amountInMicroUnits.toString());
    } else {
        // Get min bond amount
        const minValidatorBond = await api.query.staking.minValidatorBond();

        // Should atleast bond the min validator bond amount on initial bond
        if (amount.cmp(minValidatorBond) === -1) {
            const amountMsg = minValidatorBond.toBigInt() / precision;
            throw new Error(`Amount to bond must be at least: ${amountMsg.toString()} CTC (min validator bond amount)`);
        }

        bondTx = api.tx.staking.bond(amountInMicroUnits.toString(), rewardDestination);
    }

    await requireKeyringHasSufficientFunds(bondTx, stashKeyring, api, amount);
    return await signSendAndWatchCcKeyring(bondTx, api, stashKeyring);
}

export async function hasBondedEnough(keyring: CcKeyring, api: ApiPromise) {
    // Get min bond amount
    const minValidatorBond = await api.query.staking.minValidatorBond();

    // Get balance
    const balance = await getBalance(keyring.pair.address, api);

    return minValidatorBond.cmp(balance.bonded) !== 1;
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
    callerKeyring: KeyringPair,
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
    await signSendAndWatch(sudoTx, api, callerKeyring);
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
