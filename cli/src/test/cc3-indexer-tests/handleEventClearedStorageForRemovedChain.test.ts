import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventClearedStorageForRemovedChain()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    // unique integer to serve as chain id during testing
    const newChainId = Date.now();
    const newChainName = `Test Chain ${newChainId}`;
    let newChainKey = 0n;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        await api.tx.sudo
            .sudo(api.tx.supportedChains.registerChain(newChainId, newChainName, null, null, null, null, null, null))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // will fail if the query returns None
        newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
            .unwrap()
            .toBigInt();
        expect(newChainKey).toBeGreaterThan(0n);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when a supported chain is removed', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known ClearedStorageForRemovedChains entity', async () => {
            const response = await graphQLQuery(
                `query { clearedStorageForRemovedChains(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, chainKey, whoId }}}`,
            );
            expect(response.data.clearedStorageForRemovedChains.nodes).toBeTruthy();
            expect(response.data.clearedStorageForRemovedChains.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.clearedStorageForRemovedChains.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(node.whoId).toEqual(root.address);
            }
        });
    });
});
