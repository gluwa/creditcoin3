import { newApi, ApiPromise, KeyringPair, BN, MICROUNITS_PER_CTC } from '../../../../lib';
import { fundFromSudo } from '../../../integration-tests/helpers';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('Chill', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;
    let attestorAccount: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
        attestorAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        await fundFromSudo(attestorAccount.address, MICROUNITS_PER_CTC.mul(new BN(2000)));
        await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestorAccount.address).signAndSend(alice);

        // wait for at least one block b/c when registerAttestor() & chill() happen to be in the same
        // block chill() will fail b/c storage hasn't been updated yet!
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        return new Promise((resolve, reject): void => {
            // NOTE: this is signed by the random attestor account
            const unsubscribe = api.tx.attestation
                .chill(chain_Anvil2_Key, attestorAccount.address)
                .signAndSend(alice, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
