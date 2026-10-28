import { WsProvider } from '../../lib';
import { sleep } from '../utils';

interface Head {
    hash: string;
    // eslint-disable-next-line id-denylist -- Ethereum JSON-RPC field name.
    number: string;
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

    const canonicalBlock = async (head: Head) => {
        for (let attempt = 0; attempt < 40; attempt++) {
            const block = await provider.send('eth_getBlockByHash', [head.hash, false]);
            if (block !== null) {
                return block;
            }
            // Frontier's hash lookup only returns canonical blocks. The mixed-version
            // validator network can replace a notified head before we query it.
            const canonical = await provider.send('eth_getBlockByNumber', [head.number, false]);
            expect(canonical).not.toBeNull();
            if (canonical.hash !== head.hash) {
                return null;
            }
            await sleep(250);
        }
        throw new Error(`canonical head ${head.hash} was not queryable within 10 seconds`);
    };

    beforeAll(async () => {
        provider = new WsProvider((global as any).CREDITCOIN_API_URL);
        await provider.isReady;
    });

    afterAll(async () => {
        await provider.disconnect();
    });

    test('timestamps match eth_getBlockByHash and are in seconds', async () => {
        let compared = 0;
        for (let candidate = 0; compared < 2 && candidate < 10; candidate++) {
            const [head] = await nextHeads(1);

            // Check units even for a head that gets reorged out before the lookup.
            const nowSeconds = BigInt(Math.floor(Date.now() / 1000));
            const skew = BigInt(head.timestamp) - nowSeconds;
            expect(skew < 300n && skew > -300n).toBe(true);

            const block = await canonicalBlock(head);
            if (block === null) {
                continue;
            }
            expect(BigInt(head.timestamp)).toEqual(BigInt(block.timestamp));
            compared++;
        }
        expect(compared).toBe(2);
    }, 60_000);
});
