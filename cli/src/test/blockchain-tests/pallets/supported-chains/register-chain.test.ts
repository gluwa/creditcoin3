import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';
import { describeIf } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'RegisterChain', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        // unique integer to serve as chain id during testing
        const chainId = Date.now();

        const nonce = await api.rpc.system.accountNextIndex(root.address);

        // Using V1 encoding for test
        const encoding = 'V1';

        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(
                    api.tx.supportedChains.registerChain(
                        chainId,
                        `Test Chain ${chainId}`,
                        null,
                        null,
                        null,
                        null,
                        null,
                        null,
                        encoding,
                    ),
                )
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
