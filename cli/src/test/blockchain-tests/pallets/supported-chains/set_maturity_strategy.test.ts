import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee, forElapsedBlocks } from '../../../utils';

describe('SetMaturityStrategy', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let chainKey: U64;
    // unique integer to serve as chain id during testing
    const chainId = Date.now();
    const chainName = `Test Chain ${chainId}`;
    const encoding = 'V1';

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        const nonce = await api.rpc.system.accountNextIndex(root.address);
        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(chainId, chainName, null, null, null, null, null, null, encoding),
            )
            .signAndSend(root, { nonce });

        await forElapsedBlocks(api);

        // will fail if the query returns None
        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(root.address);

        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.supportedChains.setMaturityStrategy(chainKey, 'EvmFinalized'))
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
