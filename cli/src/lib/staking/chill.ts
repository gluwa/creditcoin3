import { ApiPromise } from '..';
import { CcKeyring } from '../account/keyring';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../tx';

export async function chill(stashKeyring: CcKeyring, api: ApiPromise) {
    const chillTx = api.tx.staking.chill();

    await requireKeyringHasSufficientFunds(chillTx, stashKeyring, api);
    const result = await signSendAndWatchCcKeyring(chillTx, api, stashKeyring);
    return result;
}
