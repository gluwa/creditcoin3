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

    if (BigInt(amount.toString()) < BigInt(MICROUNITS_PER_CTC)) {
        throw new Error('Amount to bond must be at least 1');
    }

    const amountInMicroUnits = amount;

    let bondTx: SubmittableExtrinsic<'promise', ISubmittableResult>;

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amountInMicroUnits.toString());
    } else {
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
