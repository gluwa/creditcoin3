import { WsProvider } from '../../lib';
import { EthHead, readSubscribedBlock } from './eth-new-heads.helpers';

// The eth `newHeads` subscription must report block timestamps in seconds, like
// `eth_getBlockBy*` and every Ethereum client. It used to send milliseconds.
describe('eth_subscribe newHeads', (): void => {
    let provider: WsProvider;

    // Resolves with the first `count` heads pushed by a fresh `newHeads` subscription.
    const nextHeads = async (count: number, timeoutMs: number): Promise<EthHead[]> => {
        const heads: EthHead[] = [];
        let subscriptionId: string | number | undefined;
        let timer: ReturnType<typeof setTimeout> | undefined;
        try {
            await new Promise<void>((resolve, reject) => {
                timer = setTimeout(() => reject(new Error('Timed out waiting for newHeads')), timeoutMs);
                provider
                    .subscribe('eth_subscription', 'eth_subscribe', ['newHeads'], (error, head: EthHead) => {
                        if (error) {
                            reject(error);
                        } else if (heads.push(head) === count) {
                            resolve();
                        }
                    })
                    .then((id) => {
                        subscriptionId = id;
                    })
                    .catch(reject);
            });
        } finally {
            clearTimeout(timer);
            if (subscriptionId !== undefined) {
                await provider.unsubscribe('eth_subscription', 'eth_unsubscribe', subscriptionId);
            }
        }
        return heads;
    };

    beforeAll(async () => {
        provider = new WsProvider((global as any).CREDITCOIN_API_URL);
        await provider.isReady;
    });

    afterAll(async () => {
        await provider.disconnect();
    });

    test('timestamps match eth_getBlockByHash and are in seconds', async () => {
        const deadline = Date.now() + 50_000;
        let compared = 0;
        while (compared < 2 && Date.now() < deadline) {
            const [head] = await nextHeads(1, deadline - Date.now());

            // Seconds, not milliseconds: within a few minutes of now. A millisecond timestamp
            // would be about 1000 times larger.
            const nowSeconds = BigInt(Math.floor(Date.now() / 1000));
            const skew = BigInt(head.timestamp) - nowSeconds;
            expect(skew < 300n && skew > -300n).toBe(true);

            const block = await readSubscribedBlock(
                (method, params) => provider.send<EthHead | null>(method, params),
                head,
                Math.max(1, Math.min(5_000, deadline - Date.now())),
            );
            if (block !== null) {
                expect(block.hash).toEqual(head.hash);
                expect(BigInt(head.timestamp)).toEqual(BigInt(block.timestamp));
                compared++;
            }
        }
        expect(compared).toEqual(2);
    }, 60_000);
});
