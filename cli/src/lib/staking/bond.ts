import { ApiPromise, BN, KeyringPair } from '..'
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx'

type RewardDestination = 'Staked' | 'Stash'

export async function bond(
    stashKeyring: KeyringPair,
    amount: BN,
    rewardDestination: RewardDestination,
    api: ApiPromise,
    extra = false
) {
    console.log(`Amount: ${amount.toString()}`)

    // TODO resupport this min amount check
    // if (amount.lt(new BN(1).mul(new BN(MICROUNITS_PER_CTC)))) {
    //   throw new Error("Amount to bond must be at least 1");
    // }

    const amountInMicroUnits = amount

    let bondTx

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amountInMicroUnits.toString())
    } else {
        bondTx = api.tx.staking.bond(
            amountInMicroUnits.toString(),
            rewardDestination
        )
    }

    await requireEnoughFundsToSend(bondTx, stashKeyring.address, api, amount)

    const result = await signSendAndWatch(bondTx, api, stashKeyring)

    return result
}

export function parseRewardDestination(
    rewardDestinationRaw: string
): RewardDestination {
    // Capitalize first letter and lowercase the rest
    const rewardDestination =
        rewardDestinationRaw.charAt(0).toUpperCase() +
        rewardDestinationRaw.slice(1).toLowerCase()

    if (rewardDestination !== 'Staked' && rewardDestination !== 'Stash') {
        throw new Error(
            "Invalid reward destination, must be one of 'Staked' or 'Stash'"
        )
    } else {
        return rewardDestination
    }
}

export function checkRewardDestination(
    rewardDestinationRaw: string
): RewardDestination {
    // Capitalize first letter and lowercase the rest
    const rewardDestination =
        rewardDestinationRaw.charAt(0).toUpperCase() +
        rewardDestinationRaw.slice(1).toLowerCase()

    if (rewardDestination !== 'Staked' && rewardDestination !== 'Stash') {
        throw new Error(
            "Invalid reward destination, must be one of 'Staked' or 'Stash'"
        )
    } else {
        return rewardDestination
    }
}
