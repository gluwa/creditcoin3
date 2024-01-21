import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { ApiPromise, BN, KeyringPair, MICROUNITS_PER_CTC } from '..';
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx';

type RewardDestination = 'Staked' | 'Stash';

export async function bond(
    stashKeyring: KeyringPair | null,
    amount: BN,
    rewardDestination: RewardDestination,
    api: ApiPromise,
    extra = false,
    proxy: string | null = null,
    proxyKeyring: KeyringPair | null = null,
    address: string | null = null,
) {
    console.log(`Amount: ${amount.toString()}`);

    if (BigInt(amount.toString()) < BigInt(MICROUNITS_PER_CTC)) {
        throw new Error('Amount to bond must be at least 1');
    }

    const amountInMicroUnits = amount;

    let bondTx: SubmittableExtrinsic<'promise', ISubmittableResult>;
    let callerAddress = stashKeyring?.address;
    let callerKeyring = stashKeyring;

    if (extra) {
        bondTx = api.tx.staking.bondExtra(amountInMicroUnits.toString());
    } else {
        bondTx = api.tx.staking.bond(amountInMicroUnits.toString(), rewardDestination);
    }

    if (proxy) {
        if (!proxyKeyring) {
            console.log('ERROR: proxy keyring not provided through $PROXY_SECRET or interactive prompt');
            process.exit(1);
        }
        if (!address) {
            console.log("ERROR: Address not supplied, provide with '--address <address>'");
            process.exit(1);
        }
        console.log(`Using proxy ${proxyKeyring.address} for address ${address}`);
        bondTx = api.tx.proxy.proxy(address, null, bondTx);
        callerAddress = proxyKeyring.address;
        callerKeyring = proxyKeyring;
    }

    // catch cases where no proxy was provided and no stash keyring was initialized
    if (!stashKeyring) {
        throw new Error('ERROR: No proxy or stash keyring was provided!');
    }
    if (!callerAddress) {
        throw new Error('ERROR: Caller address was not valid');
    }
    if (!callerKeyring) {
        throw new Error('ERROR: Caller keyring was not valid');
    }

    await requireEnoughFundsToSend(bondTx, callerAddress, api, amount);
    return await signSendAndWatch(bondTx, api, callerKeyring);
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
