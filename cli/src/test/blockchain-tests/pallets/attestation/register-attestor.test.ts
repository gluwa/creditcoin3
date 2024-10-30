import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('RegisterAttestor', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        return new Promise((resolve, reject): void => {
            const attrAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');

            // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
            const unsubscribe = api.tx.attestation
                .registerAttestor(chain_Anvil2_Key, attrAccount.address)
                .signAndSend(alice, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
