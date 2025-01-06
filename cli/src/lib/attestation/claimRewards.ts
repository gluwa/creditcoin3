import { newApi } from '../../lib/api';
import { internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorClaimRewards(options: OptionValues) {
    const rpcUrl = options.rpcUrl as string;

    const { api } = await newApi(rpcUrl);
    const txCall = api.tx.attestation
    .claimRewards();

    const signer  = options.signer as string;

    await internalSignSendAndWatchBySender(txCall, signer);
}
