import { newApi } from '../../lib/api';
import { internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttestorWithdrawUnbonded(options: OptionValues) {

    const signer = options.signer as string;
    const rpcUrl = options.rpcUrl as string;

    const { api } = await newApi(rpcUrl);


    const txCall = api.tx.attestation
    .withdrawUnbonded();

    await internalSignSendAndWatchBySender(txCall, signer);
}
