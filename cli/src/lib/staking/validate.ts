import { ApiPromise, KeyringPair } from '..';
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx';

export interface StakingPalletValidatorPrefs {
    // The validator's commission.
    commission: number;
    // Whether or not the validator is accepting more nominations.
    blocked: boolean;
}

export async function validate(account: KeyringPair, prefs: StakingPalletValidatorPrefs, api: ApiPromise) {
    console.log('Creating validate transaction with params:');

    const preferences: StakingPalletValidatorPrefs = prefs || {
        commission: 0,
        blocked: false,
    };

    console.log(`Comission: ${preferences.commission}`);
    console.log(`Blocked for new nominators: ${preferences.blocked.toString()}`);

    const validateTx = api.tx.staking.validate(preferences);

    await requireEnoughFundsToSend(validateTx, account.address, api);

    const result = await signSendAndWatch(validateTx, api, account);

    return result;
}
