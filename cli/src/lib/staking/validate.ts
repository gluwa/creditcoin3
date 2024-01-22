import { ApiPromise, KeyringPair } from '..';
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx';

export interface StakingPalletValidatorPrefs {
    // The validator's commission.
    commission: number;
    // Whether or not the validator is accepting more nominations.
    blocked: boolean;
}

export async function validate(
    account: KeyringPair,
    prefs: StakingPalletValidatorPrefs,
    api: ApiPromise,
    proxyKeyring: KeyringPair | null,
    address: string | null,
) {
    console.log('Creating validate transaction with params:');

    const preferences: StakingPalletValidatorPrefs = prefs || {
        commission: 0,
        blocked: false,
    };

    console.log(`Comission: ${preferences.commission}`);
    console.log(`Blocked for new nominators: ${preferences.blocked.toString()}`);

    let validateTx = api.tx.staking.validate(preferences);
    let callerAddress = account.address;
    let caller = account;

    if (proxyKeyring) {
        if (!address) {
            throw new Error("ERROR: Address not supplied, provide with '--address <address>'");
        }

        validateTx = api.tx.proxy.proxy(address, null, validateTx);
        callerAddress = proxyKeyring.address;
        caller = proxyKeyring;
    }

    await requireEnoughFundsToSend(validateTx, callerAddress, api);
    return await signSendAndWatch(validateTx, api, caller);
}
