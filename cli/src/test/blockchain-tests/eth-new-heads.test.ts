import { WsProvider } from '../../lib';

interface Head {
    hash: string;
    timestamp: string;
}

// The eth `newHeads` subscription must report block timestamps in seconds, like
// `eth_getBlockBy*` and every Ethereum client. It used to send milliseconds.
describe('eth_subscribe newHeads', (): void => {
    let provider: WsProvider;

    // Resolves with the first `count` heads pushed by a fresh `newHeads` subscription.
    const nextHeads = async (count: number): Promise<Head[]> => {
        const heads: Head[] = [];
        let subscriptionId: string | number | undefined;
        await new Promise<void>((resolve, reject) => {
            provider
                .subscribe('eth_subscription', 'eth_subscribe', ['newHeads'], (error, head: Head) => {
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
        if (subscriptionId !== undefined) {
            await provider.unsubscribe('eth_subscription', 'eth_unsubscribe', subscriptionId);
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
        // On a network with several authors (validator-compatibility) a head can be
        // reorged out before we look it up, and Frontier returns null for blocks that
        // are no longer on the main chain. Retrying doesn't help, so take a few heads
        // and check the ones that are still canonical.
        const heads = await nextHeads(5);
        const checked: { head: Head; block: { timestamp: string } }[] = [];
        for (const head of heads) {
            const block = await provider.send('eth_getBlockByHash', [head.hash, false]);
            if (block !== null) {
                checked.push({ head, block });
            }
        }
        expect(checked.length).toBeGreaterThanOrEqual(2);

        for (const { head, block } of checked) {
            expect(BigInt(head.timestamp)).toEqual(BigInt(block.timestamp));

            // Seconds, not milliseconds: within a few minutes of now. A millisecond timestamp
            // would be about 1000 times larger.
            const nowSeconds = BigInt(Math.floor(Date.now() / 1000));
            const skew = BigInt(head.timestamp) - nowSeconds;
            expect(skew < 300n && skew > -300n).toBe(true);
        }
    }, 60_000);
});
