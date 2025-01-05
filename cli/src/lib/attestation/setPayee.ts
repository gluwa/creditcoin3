import { newApi } from '../../lib/api';
import { internalSignSendAndWatch, internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorSetPayee(options: OptionValues) {
    const { api } = await newApi();

    const payee = options.payee as string;
    const signer = options.signer as string;

    const txCall = api.tx.attestation
    .setPayee(payee);

    await internalSignSendAndWatchBySender(txCall, signer);
}
