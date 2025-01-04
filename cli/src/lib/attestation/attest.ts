import { BN, newApi } from '../../lib';
import { internalSignSendAndWatch } from '../../lib/tx_for_fe';

export type OptionValues = Record<string, any>;

export async function callAttest(options: OptionValues) {

    const chainKey = options.chainKey as string;
    const blsPublicKey = options.blsPublicKey as string;
    const proofOfPossession = options.proofOfPossession as string;

    const { api } = await newApi();
    const txCall = api.tx.attestation
    .attest(chainKey, blsPublicKey, proofOfPossession);

    await internalSignSendAndWatch(txCall);
}
