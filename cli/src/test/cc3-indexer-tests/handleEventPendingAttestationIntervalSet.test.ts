import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventPendingAttestationIntervalSet()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: number;
    // unique integer to serve as chain id during testing
    const newChainId = Date.now();
    const newChainName = `Test Chain ${newChainId}`;
    const newInterval = randomIntBetween(10, 100);
    let newChainKey = 0;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        await api.tx.sudo.sudo(api.tx.supportedChains.registerChain(newChainId, newChainName)).signAndSend(root);
        await forElapsedBlocks(api, { minBlocks: 1 });

        // will fail if the query returns None
        newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
            .unwrap()
            .toNumber();
        expect(newChainKey).toBeGreaterThan(0);
    }, 30_000);

    afterAll(async () => {
        await api.tx.sudo.sudo(api.tx.supportedChains.removeChain(newChainKey, true)).signAndSend(root);

        await api.disconnect();
    });

    describe('when a new chain attestation interval is set', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(api.tx.attestation.setChainAttestationInterval(newChainKey, newInterval))
                .signAndSend(root);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known PendingAttestationIntervalSet entity', async () => {
            const response = await graphQLQuery(
                `query { pendingAttestationIntervalSets(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, chainKey, interval, whoId }}}`,
            );
            expect(response.data.pendingAttestationIntervalSets.nodes).toBeTruthy();
            expect(response.data.pendingAttestationIntervalSets.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.pendingAttestationIntervalSets.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.blockNumber).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.chainKey).toEqual(newChainKey);
                expect(node.interval).toEqual(newInterval);
                expect(node.whoId).toEqual(root.address);
            }
        });
    });
});
