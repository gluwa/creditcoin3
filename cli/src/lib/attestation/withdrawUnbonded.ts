import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';

export async function callAttestorWithdrawUnbonded() {
    const { api } = await newApi();

    const txCall = api.tx.attestation
    .withdrawUnbonded();

    await internalSignSendAndWatch(txCall);
}
