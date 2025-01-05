import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch, internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;
export async function callAttestorUnregisterAttestor(options: OptionValues) {
    const { api } = await newApi();

    const chainKey = options.chainKey as string;
    const attestorId = options.attestorId as string;
    const signer = options.signer as string;

    const txCall = api.tx.attestation
    .unregisterAttestor(chainKey, attestorId);

    await internalSignSendAndWatchBySender(txCall, signer);
}
