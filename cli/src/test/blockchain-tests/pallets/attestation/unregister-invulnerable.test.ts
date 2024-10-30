import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('UnregisterInvulnerable', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let attestorAccount: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // NOTE: Insert a random invulnerable for the Anvil2 chain
        attestorAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        await api.tx.sudo
            .sudo(api.tx.attestation.registerInvulnerable(chain_Anvil2_Key, attestorAccount.address))
            .signAndSend(root);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.attestation.unregisterInvulnerable(chain_Anvil2_Key, attestorAccount.address))
                .signAndSend(root, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
