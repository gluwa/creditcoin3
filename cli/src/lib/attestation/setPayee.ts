import { newApi } from '../../lib/api';
import { internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorSetPayee(options: OptionValues) {
    const payee = options.payee as string;
    const signer = options.signer as string;
    const rpcUrl = options.rpcUrl as string;

    const { api } = await newApi(rpcUrl);

    const txCall = api.tx.attestation
    .setPayee(payee);

    await internalSignSendAndWatchBySender(txCall, signer);
}
