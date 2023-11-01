import { ApiPromise, KeyringPair } from '..';
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx';

export async function chill(controllerKeyring: KeyringPair, api: ApiPromise) {
    const chillTx = api.tx.staking.chill();
    await requireEnoughFundsToSend(chillTx, controllerKeyring.address, api);
    const result = await signSendAndWatch(chillTx, api, controllerKeyring);
    return result;
}
