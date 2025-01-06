import { newApi } from '../../lib/api';
import { internalSignSendAndWatchBySender } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttest(options: OptionValues) {
    const chainKey = options.chainKey as string;
    const blsPublicKey = options.blsPublicKey as string;
    const proofOfPossession = options.proofOfPossession as string;
    const signer  = options.signer as string;
    const rpcUrl = options.rpcUrl as string;

    const { api } = await newApi(rpcUrl);
    const txCall = api.tx.attestation
    .attest(chainKey, blsPublicKey, proofOfPossession);

    await internalSignSendAndWatchBySender(txCall, signer);
}
