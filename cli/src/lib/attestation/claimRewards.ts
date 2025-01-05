import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch, internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorClaimRewards(options: OptionValues) {
    const { api } = await newApi();
    const txCall = api.tx.attestation
    .claimRewards();

    const signer  = options.signer as string;

    await internalSignSendAndWatchBySender(txCall, signer);
}
