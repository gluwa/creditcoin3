import { newApi } from '../../lib/api';
import { internalSignSendAndWatch, internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorWithdrawUnbonded(options: OptionValues) {
    const { api } = await newApi();

    const signer = options.signer as string;

    const txCall = api.tx.attestation
    .withdrawUnbonded();

    await internalSignSendAndWatchBySender(txCall, signer);
}
