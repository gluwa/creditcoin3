import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorSetPayee(options: OptionValues) {
    const { api } = await newApi();

    const payee = options.payee as string;

    const txCall = api.tx.attestation
    .setPayee(payee);

    await internalSignSendAndWatch(txCall);
}
