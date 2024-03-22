import { ApiPromise, hasBondedEnough } from '..';
import { CcKeyring } from '../account/keyring';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../tx';
export interface StakingPalletValidatorPrefs {
    // The validator's commission.
    commission: number;
    // Whether or not the validator is accepting more nominations.
    blocked: boolean;
}

export async function validate(stashKeyring: CcKeyring, prefs: StakingPalletValidatorPrefs, api: ApiPromise) {
    console.log('Creating validate transaction with params:');

    const enoughBond: boolean = await hasBondedEnough(stashKeyring, api);
    if (!enoughBond) {
        throw new Error('Cannot start validating, not enough bonded.');
    }

    const preferences: StakingPalletValidatorPrefs = prefs || {
        commission: 0,
        blocked: false,
    };

    console.log(`Comission: ${preferences.commission}`);
    console.log(`Blocked for new nominators: ${preferences.blocked.toString()}`);

    const validateTx = api.tx.staking.validate(preferences);

    await requireKeyringHasSufficientFunds(validateTx, stashKeyring, api);
    return await signSendAndWatchCcKeyring(validateTx, api, stashKeyring);
}
