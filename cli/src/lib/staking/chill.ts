import { ApiPromise, KeyringPair } from '..';
import { requireEnoughFundsToSend, signSendAndWatch } from '../tx';

export async function chill(
    controllerKeyring: KeyringPair | null,
    api: ApiPromise,
    proxyKeyring: KeyringPair | null,
    address: string,
) {
    let chillTx = api.tx.staking.chill();
    let callerAddress = controllerKeyring?.address;
    let callerKeyring = controllerKeyring;

    if (proxyKeyring) {
        chillTx = api.tx.proxy.proxy(address, null, chillTx);
        callerAddress = proxyKeyring.address;
        callerKeyring = proxyKeyring;
    }

    if (!callerAddress) {
        throw new Error('ERROR: keyring not initialized and proxy not selected');
    }

    if (!callerKeyring) {
        throw new Error('ERROR: keyring not initialized and proxy not selected');
    }

    console.log(callerAddress);
    await requireEnoughFundsToSend(chillTx, callerAddress, api);
    const result = await signSendAndWatch(chillTx, api, callerKeyring);
    return result;
}
