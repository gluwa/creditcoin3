import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { ApiPromise, BN, KeyringPair, MICROUNITS_PER_CTC } from '..';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring, signSendAndWatch } from '../tx';
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

    if (BigInt(amount.toString()) < BigInt(MICROUNITS_PER_CTC)) {
        throw new Error('Amount to bond must be at least 1');
    }

    const amountInMicroUnits = amount;

    let bondTx: SubmittableExtrinsic<'promise', ISubmittableResult>;

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amountInMicroUnits.toString());
    } else {
        // Get min bond amount
        let min_bond_amount = await api.query.staking.minValidatorBond();
    
        // Should atleast bond the min validator bond amount on initial bond
        if (BigInt(amount.toString()) < (min_bond_amount.toNumber() * 1e18)) {
            throw new Error('Amount to bond must be at least the minimum validator bond amount');
        }

        bondTx = api.tx.staking.bond(amountInMicroUnits.toString(), rewardDestination);
    }

    await requireKeyringHasSufficientFunds(bondTx, stashKeyring, api, amount);
    return await signSendAndWatchCcKeyring(bondTx, api, stashKeyring);
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
    const configTx = await api.tx.staking.setStakingConfigs(
        setStakingConfigOp(minNomitatorBond), 
        setStakingConfigOp(minValidatorBond), 
        setStakingConfigOp(maxNominatorCount), 
        setStakingConfigOp(maxValidatorCount), 
        setStakingConfigOp(chillThreshold), 
        setStakingConfigOp(minCommission)
    )

    const sudoTx = api.tx.sudo.sudo(configTx);
    await signSendAndWatch(sudoTx, api, callerKeyring);
}

function setStakingConfigOp(op: any): any {
    if (op == 0) {
        op = { Remove: null }
    } else if (op == null) {
        op = { Noop: null }
    } else {
        op = { Set: op }
    }

    return op;
}