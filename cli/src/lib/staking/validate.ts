import { ApiPromise } from '..';
import { CcKeyring } from '../account/keyring';
import { requireEnoughFundsToSend, signSendAndWatchCcKeyring } from '../tx';

export interface StakingPalletValidatorPrefs {
    // The validator's commission.
    commission: number;
    // Whether or not the validator is accepting more nominations.
    blocked: boolean;
}

export async function validate(stashKeyring: CcKeyring, prefs: StakingPalletValidatorPrefs, api: ApiPromise) {
    console.log('Creating validate transaction with params:');

    const preferences: StakingPalletValidatorPrefs = prefs || {
        commission: 0,
        blocked: false,
    };

    console.log(`Comission: ${preferences.commission}`);
    console.log(`Blocked for new nominators: ${preferences.blocked.toString()}`);

    const validateTx = api.tx.staking.validate(preferences);

    await requireEnoughFundsToSend(validateTx, stashKeyring.pair.address, api);
    return await signSendAndWatchCcKeyring(validateTx, api, stashKeyring);
}
