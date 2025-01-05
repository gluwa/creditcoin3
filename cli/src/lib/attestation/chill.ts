import { newApi } from '../../lib/api';
import { internalSignSendAndWatch, internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callChillAttestor(options: OptionValues) {
    const { api } = await newApi();

    const chainKey = options.chainKey as string;
    const signer  = options.signer as string;

    const txCall = api.tx.attestation
    .chill(chainKey);

    await internalSignSendAndWatchBySender(txCall, signer);
}
