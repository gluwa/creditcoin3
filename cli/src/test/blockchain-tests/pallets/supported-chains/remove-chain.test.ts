import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee, forElapsedBlocks } from '../../../utils';

describe('RemoveChain', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let chainKey: U64;
    // unique integer to serve as chain id during testing
    const chainId = Date.now();
    const chainName = `Test Chain ${chainId}`;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        await api.tx.sudo.sudo(api.tx.supportedChains.registerChain(chainId, chainName)).signAndSend(root);

        await forElapsedBlocks(api);

        // will fail if the query returns None
        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.supportedChains.removeChain(chainKey, true))
                .signAndSend(root, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
