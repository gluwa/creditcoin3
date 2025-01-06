import { newApi } from '../../lib/api';
import { internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callChillAttestor(options: OptionValues) {
    const chainKey = options.chainKey as string;
    const signer  = options.signer as string;
    const rpcUrl = options.rpcUrl as string;

    const { api } = await newApi(rpcUrl);

    const txCall = api.tx.attestation
    .chill(chainKey);

    await internalSignSendAndWatchBySender(txCall, signer);
}
