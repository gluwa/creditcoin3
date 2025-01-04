import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';

export async function callAttestorClaimRewards() {
    const { api } = await newApi();
    const txCall = api.tx.attestation
    .claimRewards();

    await internalSignSendAndWatch(txCall);
}
