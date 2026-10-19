import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks, isManualSealing, randomIntBetween, sealBlock, sleep } from '../utils';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery, indexedRecordQuery } from './common';

export async function waitForAppliedSampleSize(
    api: ApiPromise,
    chainKey: bigint,
    targetSampleSize: number,
    afterBlock: bigint,
    timeoutMs = 240_000,
): Promise<{ blockNumber: bigint; eventId: string }> {
    const sealing = await isManualSealing(api);
    const deadline = Date.now() + timeoutMs;
    let current = -1;
    while (Date.now() < deadline) {
        const finalizedHash = await api.rpc.chain.getFinalizedHead();
        const finalizedState = await api.at(finalizedHash);
        current = (await finalizedState.query.attestation.targetSampleSize(chainKey)).toNumber();
        if (current === targetSampleSize) {
            const finalizedBlock = (await api.rpc.chain.getHeader(finalizedHash)).number.toBigInt();
            // Correlate the applied value with the exact natural epoch event, excluding the
            // chain-registration default and unrelated changes from other suites.
            for (let block = afterBlock + 1n; block <= finalizedBlock; block++) {
                if (Date.now() >= deadline) {
                    throw new Error(`timed out locating the sample-size application event after ${afterBlock}`);
                }
                const state = await api.at(await api.rpc.chain.getBlockHash(block.toString()));
                const events = await state.query.system.events();
                const index = events.findIndex(
                    ({ event, phase }) =>
                        phase.isInitialization &&
                        api.events.attestation.TargetSampleSizeChanged.is(event) &&
                        event.data[0].toBigInt() === chainKey &&
                        event.data[1].toNumber() === targetSampleSize,
                );
                if (index >= 0) {
                    if ((await state.query.attestation.targetSampleSize(chainKey)).toNumber() !== targetSampleSize) {
                        throw new Error(`sample-size event at block ${block} disagrees with its on-chain state`);
                    }
                    return { blockNumber: block, eventId: `${block}-${index}` };
                }
            }
            throw new Error(`sample size ${targetSampleSize} applied without its matching event after ${afterBlock}`);
        }
        if (sealing) {
            await sealBlock(api);
        } else {
            await sleep(api.consts.babe.expectedBlockTime.toNumber());
        }
    }
    throw new Error(
        `chain ${chainKey} sample size remained ${current}, expected ${targetSampleSize} after ${timeoutMs}ms`,
    );
}

describe('handleEventTargetSampleSizeChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    let applicationBlock: bigint;
    let applicationEventId: string;
    let targetSampleSize_Anvil1 = 0;
    let targetSampleSize_Anvil2 = 0;
    const newTargetSampleSize = randomIntBetween(250, 512);
    // unique integer to serve as chain id during testing
    const newChainId = BigInt(Date.now());
    const newChainName = `Test Chain ${newChainId}`;
    const encoding = 'V1';
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
                    encoding,
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
                ) { nodes { id, at, chainKey, chainName, chainId, chainEncoding }}}`,
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
            // Target sample size applies at a BABE epoch, independently of staking eras.
            // Wait for finalized application; each GraphQL query then waits for indexing.
            const applied = await waitForAppliedSampleSize(api, newChainKey, newTargetSampleSize, startingBlock);
            applicationBlock = applied.blockNumber;
            applicationEventId = applied.eventId;
        }, 300_000);

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
                expect(node.id).toBeTruthy();
                expect(node.id).toEqual(applicationEventId);

                expect(BigInt(node.blockNumber)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.blockNumber)).toEqual(applicationBlock);
                expect(node.whoId).toHaveLength(0);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(node.eventNewTargetSampleSize).toEqual(newTargetSampleSize);

                // query each node individually to cover this endpoint too
                const response2 = await indexedRecordQuery(
                    `query { targetSampleSizeChanged(id: "${node.id}") { id, chainKey, blockNumber, whoId, eventNewTargetSampleSize }}`,
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
