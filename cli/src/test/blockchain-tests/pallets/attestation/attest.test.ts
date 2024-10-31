import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { randomFundedAccount } from '../../../integration-tests/helpers';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('Attest', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // NOTE: Alice is the STASH for a random attestor on the Anvil2 chain
        attestor = await randomFundedAccount(api, root);
        await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(alice);

        // wait for Attestors storage item to be updated!
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const blsSecretKey = WasmPrivateKey.generate(attestor.secret);
        const blsPublicKey = blsSecretKey.public_key().as_bytes();
        const proofOfPossession = blsSecretKey.sign(blsPublicKey);

        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.attestation
                .attest(chain_Anvil2_Key, blsPublicKey, proofOfPossession.as_bytes())
                .signAndSend(attestor.keyring, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
