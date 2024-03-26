import { ApiPromise, hasBondedEnough } from '..';
import { CcKeyring, delegateAddress } from '../account/keyring';
import { getBalance } from '../balance';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../tx';
export interface StakingPalletValidatorPrefs {
    // The validator's commission.
    commission: number;
    // Whether or not the validator is accepting more nominations.
    blocked: boolean;
}

export async function validate(stashKeyring: CcKeyring, prefs: StakingPalletValidatorPrefs, api: ApiPromise) {
    console.log('Creating validate transaction with params:');

    const address = delegateAddress(stashKeyring);
    const balance = await getBalance(address, api);

    // Check if address has bonded enough
    await hasBondedEnough(balance.bonded, api);

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
