import { ApiPromise } from '..';
import { CcKeyring } from '../account/keyring';
import { requireEnoughFundsToSend, signSendAndWatchCcKeyring } from '../tx';

export async function chill(stashKeyring: CcKeyring, api: ApiPromise) {
    const chillTx = api.tx.staking.chill();

    await requireEnoughFundsToSend(chillTx, stashKeyring.pair.address, api);
    const result = await signSendAndWatchCcKeyring(chillTx, api, stashKeyring);
    return result;
}
