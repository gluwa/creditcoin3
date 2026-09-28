import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleMaturityStrategySet()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const newChainName = `Test Chain ${newChainId}`;
    const encoding = 'V1';
    // Registered with one strategy, then moved to another. Both are valid; `register_chain`
    // rejects an unknown strategy and `set_maturity_strategy` rejects the current one, so the
    // pair has to differ.
    const initialStrategy = 'EvmSafe';
    const newStrategy = 'RpcFinalized';
    let newChainKey = 0n;

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

    describe('when the maturity strategy of a registered chain is changed', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(
                    api.tx.supportedChains.registerChain(
                        newChainId,
                        newChainName,
                        null,
                        null,
                        null,
                        null,
                        null,
                        null,
                        encoding,
                        initialStrategy,
                    ),
                )
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 1 });

            // will fail if the query returns None
            newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
                .unwrap()
                .toBigInt();
            expect(newChainKey).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.setMaturityStrategy(newChainKey, newStrategy))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 90_000);

        it('chain state holds the new strategy', async () => {
            const chain = (await api.query.supportedChains.supportedChains(newChainKey)).unwrap();
            expect(chain.maturityStrategy.toString()).toEqual(newStrategy);
        });

        it('graphQL returns known MaturityStrategySet entity', async () => {
            const response = await graphQLQuery(
                `query {
                    maturityStrategySets(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainId, maturityStrategy, whoId }}}`,
            );
            expect(response.data.maturityStrategySets.nodes).toBeTruthy();
            expect(response.data.maturityStrategySets.nodes.length).toEqual(1);

            for (const node of response.data.maturityStrategySets.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.at)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(BigInt(node.chainId)).toEqual(newChainId);
                expect(node.maturityStrategy).toEqual(newStrategy);
                expect(node.whoId).toEqual(root.address);
            }
        });

        // The registration row is updated in place rather than duplicated: a consumer reading
        // SupportedChain must see the live strategy, not the one it was registered with.
        it('graphQL SupportedChain entity carries the new strategy', async () => {
            const response = await graphQLQuery(
                `query {
                    supportedChains(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                    ) { nodes { id, chainKey, chainName, chainId, chainEncoding, maturityStrategy }}}`,
            );
            expect(response.data.supportedChains.nodes).toBeTruthy();
            expect(response.data.supportedChains.nodes.length).toEqual(1);

            const node = response.data.supportedChains.nodes[0];
            expect(BigInt(node.chainKey)).toEqual(newChainKey);
            expect(node.chainName).toEqual(newChainName);
            expect(BigInt(node.chainId)).toEqual(newChainId);
            expect(node.chainEncoding).toEqual(encoding);
            expect(node.maturityStrategy).toEqual(newStrategy);
        });
    });
});
