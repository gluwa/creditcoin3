import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventTargetSampleSizeChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    let targetSampleSize_Anvil1 = 0;
    let targetSampleSize_Anvil2 = 0;
    const newTargetSampleSize = 14;
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const newChainName = `Test Chain ${newChainId}`;
    let newChainKey = 0n;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const response = await graphQLQuery(
            `query { attestationChainData(orderBy: CHAIN_KEY_ASC, last: 10) { nodes { id, chainKey, targetSampleSize }}}`,
        );
        for (const node of response.data.attestationChainData.nodes) {
            if (node.chainKey === chain_Anvil1_Key.toString()) {
                targetSampleSize_Anvil1 = node.targetSampleSize;
            }

            if (node.chainKey === chain_Anvil2_Key.toString()) {
                targetSampleSize_Anvil2 = node.targetSampleSize;
            }
        }
        expect(targetSampleSize_Anvil1).toBeGreaterThan(0);
        expect(targetSampleSize_Anvil2).toBeGreaterThan(0);

        // new chain to be used for testing
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
                    null,
                    null,
                ),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // will fail if the query returns None
        newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
            .unwrap()
            .toBigInt();
        expect(newChainKey).toBeGreaterThan(0);

        // there should be a SupportedChain entity for this new chain
        await forElapsedBlocks(api, { minBlocks: 3 });
        const response2 = await graphQLQuery(
            `query {
                supportedChains(
                    filter: { chainKey: { equalTo: "${newChainKey}" }},
                    last: 1,
                ) { nodes { id, at, chainKey, chainName, chainId }}}`,
        );
        expect(response2.data.supportedChains.nodes).toBeTruthy();
        expect(response2.data.supportedChains.nodes.length).toEqual(1);
    }, 60_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
    });

    describe('when target sample size for a chain changes', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);

            await api.tx.sudo
                .sudo(api.tx.attestation.setTargetSampleSize(newChainKey, newTargetSampleSize))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            // wait for txn to make it on chain & indexer to ingest the block
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known TargetSampleSizeChanged', async () => {
            const response = await graphQLQuery(
                `query { targetSampleSizeChangeds(
                    orderBy: BLOCK_NUMBER_ASC,
                    filter: { chainKey: { equalTo: "${newChainKey}" }},
                    last: 1,
                ) { nodes { id, blockNumber, whoId, chainKey, eventNewTargetSampleSize }}}`,
            );
            expect(response.data.targetSampleSizeChangeds.nodes).toBeTruthy();
            expect(response.data.targetSampleSizeChangeds.nodes.length).toEqual(1);

            for (const node of response.data.targetSampleSizeChangeds.nodes) {
                expect(BigInt(node.blockNumber)).toBeGreaterThanOrEqual(startingBlock);
                expect(node.whoId).toEqual(root.address);
                expect(node.eventNewTargetSampleSize).toEqual(newTargetSampleSize);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { targetSampleSizeChanged(id: "${node.id}") { id, whoId, chainKey, blockNumber, eventNewTargetSampleSize }}`,
                );
                expect(response2.data.targetSampleSizeChanged).toBeTruthy();
                expect(response2.data.targetSampleSizeChanged.id).toEqual(node.id);
                expect(response2.data.targetSampleSizeChanged.chainKey).toEqual(node.chainKey);
                expect(response2.data.targetSampleSizeChanged.whoId).toEqual(node.whoId);
                expect(response2.data.targetSampleSizeChanged.blockNumber).toEqual(node.blockNumber);
                expect(response2.data.targetSampleSizeChanged.eventNewTargetSampleSize).toEqual(
                    node.eventNewTargetSampleSize,
                );
            }
        });

        it('graphQL returns updated AttestationChainData', async () => {
            const response = await graphQLQuery(
                `query { attestationChainData(
                    orderBy: CHAIN_KEY_ASC,
                    filter: { chainKey: { equalTo: "${newChainKey}" }},
                    last: 1,
                ) { nodes { id, chainKey, targetSampleSize }}}`,
            );
            expect(response.data.attestationChainData.nodes).toBeTruthy();
            expect(response.data.attestationChainData.nodes.length).toEqual(1);

            for (const node of response.data.attestationChainData.nodes) {
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(node.targetSampleSize).toEqual(newTargetSampleSize);
            }
        });
    });
});
