import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleSupportedChainRemoved()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    let defaultMaturityStrategy: string;
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const newChainName = `Test Chain ${newChainId}`;
    const encoding = 'V1';
    let newChainKey = 0n;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        defaultMaturityStrategy = api.consts.supportedChains.defaultMaturityStrategy.toString();

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
                ),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // will fail if the query returns None
        newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
            .unwrap()
            .toBigInt();
        expect(newChainKey).toBeGreaterThan(0n);

        // there should be a SupportedChain entity for this new chain
        await forElapsedBlocks(api, { minBlocks: 3 });
        const response = await graphQLQuery(
            `query {
                supportedChains(
                    filter: { chainKey: { equalTo: "${newChainKey}" }},
                    last: 1,
                ) { nodes { id, at, chainKey, chainName, chainId, chainEncoding, maturityStrategy }}}`,
        );
        expect(response.data.supportedChains.nodes).toBeTruthy();
        expect(response.data.supportedChains.nodes.length).toEqual(1);

        for (const node of response.data.supportedChains.nodes) {
            expect(node.id).toBeTruthy();
            // note: inspecting only last record
            expect(BigInt(node.chainKey)).toEqual(newChainKey);
            expect(node.chainName).toEqual(newChainName);
            expect(BigInt(node.chainId)).toEqual(newChainId);
            expect(node.chainEncoding).toEqual(encoding);
            expect(node.maturityStrategy).toEqual(defaultMaturityStrategy);
        }
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when a supported chain is removed', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known ChainRemoved entity', async () => {
            const response = await graphQLQuery(
                `query {
                    chainRemoveds(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainName, chainId, chainEncoding, maturityStrategy, whoId }}}`,
            );
            expect(response.data.chainRemoveds.nodes).toBeTruthy();
            expect(response.data.chainRemoveds.nodes.length).toEqual(1);

            for (const node of response.data.chainRemoveds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.at)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(node.chainName).toEqual(newChainName);
                expect(BigInt(node.chainId)).toEqual(newChainId);
                expect(node.chainEncoding).toEqual(encoding);
                expect(node.maturityStrategy).toEqual(defaultMaturityStrategy);
                expect(node.whoId).toEqual(root.address);
            }
        });

        it('known SupportedChain entity should be removed', async () => {
            const response = await graphQLQuery(
                `query {
                    supportedChains(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainName, chainId, chainEncoding, maturityStrategy }}}`,
            );
            expect(response.data.supportedChains.nodes).toBeTruthy();
            expect(response.data.supportedChains.nodes.length).toEqual(0);
        });

        it('known AttestationChainData entity should be removed', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, chainKey }}}`,
            );
            expect(response.data.attestationChainData.nodes).toBeTruthy();
            expect(response.data.attestationChainData.nodes.length).toEqual(0);
        });
    });
});
