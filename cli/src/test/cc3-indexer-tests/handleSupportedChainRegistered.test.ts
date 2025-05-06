import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleSupportedChainRegistered()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: number;
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const newChainName = `Test Chain ${newChainId}`;
    let newChainKey = 0;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
    });

    describe('when a new chain is registered', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.registerChain(newChainId, newChainName))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 1 });

            // will fail if the query returns None
            newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
                .unwrap()
                .toNumber();
            expect(newChainKey).toBeGreaterThan(0);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known ChainRegistered entity', async () => {
            const response = await graphQLQuery(
                `query {
                    chainRegistereds(
                        filter: { chainKey: { equalTo: ${newChainKey} }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainName, chainId, whoId }}}`,
            );
            expect(response.data.chainRegistereds.nodes).toBeTruthy();
            expect(response.data.chainRegistereds.nodes.length).toEqual(1);

            for (const node of response.data.chainRegistereds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.at).toBeGreaterThanOrEqual(startingBlock);
                expect(node.chainKey).toEqual(newChainKey);
                expect(node.chainName).toEqual(newChainName);
                expect(BigInt(node.chainId)).toEqual(BigInt(newChainId));
                expect(node.whoId).toEqual(root.address);
            }
        });

        it('graphQL returns known SupportedChain entity', async () => {
            const response = await graphQLQuery(
                `query {
                    supportedChains(
                        filter: { chainKey: { equalTo: ${newChainKey} }},
                        last: 1,
                    ) { nodes { id, chainKey, chainName, chainId }}}`,
            );
            expect(response.data.supportedChains.nodes).toBeTruthy();
            expect(response.data.supportedChains.nodes.length).toEqual(1);

            for (const node of response.data.supportedChains.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.chainKey).toEqual(newChainKey);
                expect(node.chainName).toEqual(newChainName);
                expect(BigInt(node.chainId)).toEqual(newChainId);
            }
        });
    });
});
