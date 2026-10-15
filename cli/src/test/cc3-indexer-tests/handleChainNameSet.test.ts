import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleChainNameSet()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const oldChainName = `Test Chain ${newChainId}`;
    const newChainName = `Renamed Chain ${newChainId}`;
    const encoding = 'V1';
    const strategy = 'EvmSafe';
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

    describe('when a registered chain is renamed', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(
                    api.tx.supportedChains.registerChain(
                        newChainId,
                        oldChainName,
                        null,
                        null,
                        null,
                        null,
                        null,
                        null,
                        encoding,
                        strategy,
                    ),
                )
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 1 });

            // will fail if the query returns None
            newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, oldChainName))
                .unwrap()
                .toBigInt();
            expect(newChainKey).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.setChainName(newChainKey, newChainName))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 90_000);

        it('chain state holds the new name and the lookup follows it', async () => {
            const chain = (await api.query.supportedChains.supportedChains(newChainKey)).unwrap();
            expect(chain.chainName.toUtf8()).toEqual(newChainName);

            expect((await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, oldChainName)).isNone).toBe(
                true,
            );
            expect(
                (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName)).unwrap().toBigInt(),
            ).toEqual(newChainKey);
        });

        it('graphQL returns known ChainNameSet entity', async () => {
            const response = await graphQLQuery(
                `query {
                    chainNameSets(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainId, oldChainName, chainName, whoId }}}`,
            );
            expect(response.data.chainNameSets.nodes).toBeTruthy();
            expect(response.data.chainNameSets.nodes.length).toEqual(1);

            for (const node of response.data.chainNameSets.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.at)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(BigInt(node.chainId)).toEqual(newChainId);
                expect(node.oldChainName).toEqual(oldChainName);
                expect(node.chainName).toEqual(newChainName);
                expect(node.whoId).toEqual(root.address);
            }
        });

        // The registration row is updated in place rather than duplicated.
        it('graphQL SupportedChain entity carries the new name', async () => {
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
            expect(node.maturityStrategy).toEqual(strategy);
        });
    });
});
