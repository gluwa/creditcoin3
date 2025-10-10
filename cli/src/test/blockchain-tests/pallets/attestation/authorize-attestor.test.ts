import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('authorizeAttestor', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let alice: KeyringPair;
    let attestorAccount: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
        attestorAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        const nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestorAccount.address)
            .signAndSend(alice, { nonce });

        await forElapsedBlocks(api, { minBlocks: 1 });
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(root.address);
        return new Promise((resolve, reject): void => {
            // note: using chain Anvil2 b/c this may lead to side effects in other test scenarios
            const authorize = api.tx.sudo
                .sudo(api.tx.attestation.authorizeAttestor(chain_Anvil2_Key, attestorAccount.address))
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, authorize, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
