import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil3_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks, sleep } from '../utils';
import { graphQLQuery } from './common';

// Poll the indexer until the latest reversion for `chainKey` is fully applied
// (status === 'complete'). Returns once settled or throws after the timeout so a
// genuine indexer failure still surfaces as a test failure rather than a hang.
async function waitForReversionComplete(chainKey: bigint, timeoutMs = 90_000, intervalMs = 2_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    let lastStatus = '<none>';

    while (Date.now() < deadline) {
        const response = await graphQLQuery(
            `query {
                revertedAttestationChainTos(
                    filter: { chainKey: { equalTo: "${chainKey}" }},
                    last: 1
                ) {
                    nodes {
                        status
                    }
                }
            }`,
        );

        const node = response?.data?.revertedAttestationChainTos?.nodes?.[0];
        if (node) {
            lastStatus = node.status;
            if (node.status === 'complete') {
                return;
            }
            if (node.status === 'failed') {
                throw new Error(`Indexer reported reversion status 'failed' for chainKey=${chainKey}`);
            }
        }

        await sleep(intervalMs);
    }

    throw new Error(
        `Timed out after ${timeoutMs}ms waiting for reversion to complete for chainKey=${chainKey} (last status: ${lastStatus})`,
    );
}

describe('handleEventRevertedAttestationChainTo()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let blockBeforeRevert: bigint;
    let timestampBeforeRevert: bigint;

    let checkpointHeightToRevertTo: bigint;
    let checkpointDigestToRevertTo: string;
    const chainKey = BigInt(chain_Anvil3_Key);

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        let checkpoints: { blockNumber: string; digest: string }[] = [];

        while (checkpoints.length < 2) {
            await forElapsedBlocks(api, { minBlocks: 2 });

            const response = await graphQLQuery(
                `query {
                    checkpoints(
                        filter: { chainKey: { equalTo: "${chainKey}" }},
                        orderBy: BLOCK_NUMBER_ASC
                    ) {
                        nodes {
                            id
                            blockNumber
                            digest
                        }
                    }
                }`,
            );

            checkpoints = response.data.checkpoints.nodes;
        }

        expect(checkpoints.length).toBeGreaterThanOrEqual(2);

        // Revert to genesis checkpoint
        checkpointHeightToRevertTo = BigInt(0);
        checkpointDigestToRevertTo = checkpoints[0].digest;
    }, 2_000_000); // Need timeout long enough to generate first non-genesis checkpoint

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when the attestation chain is reverted to a checkpoint', () => {
        beforeAll(async () => {
            blockBeforeRevert = BigInt((await getChainStatus(api)).bestNumber);
            expect(blockBeforeRevert).toBeGreaterThan(0n);
            timestampBeforeRevert = (await api.query.timestamp.now()).toBigInt();

            await api.tx.sudo
                .sudo(api.tx.attestation.revertTo(chainKey, checkpointHeightToRevertTo))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            // A fixed block wait races the indexer: the assertions below query the
            // indexer's GraphQL state, which is populated asynchronously by
            // `handleEventRevertedAttestationChainTo`. Poll until that handler has
            // finished applying the reversion (status === 'complete') so the reads
            // are deterministic regardless of how quickly the indexer catches up.
            await waitForReversionComplete(chainKey);
        }, 120_000);

        it('graphQL returns known RevertedAttestationChainTo entity', async () => {
            const response = await graphQLQuery(
                `query {
                    revertedAttestationChainTos(
                        filter: { chainKey: { equalTo: "${chainKey}" }},
                        last: 1
                    ) {
                        nodes {
                            id
                            blockNumber
                            date
                            chainKey
                            checkpointHeight
                            digest
                            status
                        }
                    }
                }`,
            );

            expect(response.data.revertedAttestationChainTos.nodes).toBeTruthy();
            expect(response.data.revertedAttestationChainTos.nodes.length).toEqual(1);

            for (const node of response.data.revertedAttestationChainTos.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.blockNumber)).toBeGreaterThanOrEqual(blockBeforeRevert);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.chainKey).toEqual(chainKey.toString());
                expect(BigInt(node.checkpointHeight)).toEqual(checkpointHeightToRevertTo);
                expect(node.digest).toEqual(checkpointDigestToRevertTo);
                expect(node.status).toEqual('complete');
            }
        });

        it('removes checkpoints above checkpointHeight', async () => {
            // Filter to checkpoints above the reverted-to height, then assert each
            // one post-dates the reversion in both chain time and block height. A
            // successful reversion removes every such row, so the loop asserts on
            // an empty set; any survivor must have been (re)indexed after the revert.
            const response = await graphQLQuery(
                `query {
                    checkpoints(
                        filter: {
                            chainKey: { equalTo: "${chainKey}" },
                            blockNumber: { greaterThan: "${checkpointHeightToRevertTo}" }
                        },
                        orderBy: BLOCK_NUMBER_ASC
                    ) {
                        nodes {
                            id
                            blockNumber
                            atBlockNumber
                            timestamp
                            digest
                        }
                    }
                }`,
            );

            expect(response.data.checkpoints.nodes).toBeTruthy();

            for (const node of response.data.checkpoints.nodes) {
                expect(BigInt(node.timestamp)).toBeGreaterThan(timestampBeforeRevert);
                expect(BigInt(node.atBlockNumber)).toBeGreaterThan(blockBeforeRevert);
            }
        });

        it('removes attestations above checkpointHeight', async () => {
            // Filter to attestations above the reverted-to height, then assert each
            // one post-dates the reversion in chain time. A successful reversion
            // removes every such row, so the loop asserts on an empty set; any
            // survivor must have been (re)indexed after the revert.
            const response = await graphQLQuery(
                `query {
                    attestations(
                        filter: {
                            chainKey: { equalTo: "${chainKey}" },
                            headerNumber: { greaterThan: "${checkpointHeightToRevertTo}" }
                        },
                        orderBy: HEADER_NUMBER_ASC
                    ) {
                        nodes {
                            id
                            headerNumber
                            timestamp
                            digest
                        }
                    }
                }`,
            );

            expect(response.data.attestations.nodes).toBeTruthy();

            for (const node of response.data.attestations.nodes) {
                expect(BigInt(node.timestamp)).toBeGreaterThan(timestampBeforeRevert);
            }
        });

        it('updates AttestationChainData to the reverted checkpoint', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(
                        filter: { chainKey: { equalTo: "${chainKey}" }},
                        last: 1
                    ) {
                        nodes {
                            id
                            chainKey
                            lastCheckpointHeaderNumber
                            lastAttestedHeaderNumber
                            lastAttestedDigest
                        }
                    }
                }`,
            );

            expect(response.data.attestationChainData.nodes).toBeTruthy();
            expect(response.data.attestationChainData.nodes.length).toEqual(1);

            const node = response.data.attestationChainData.nodes[0];
            expect(BigInt(node.chainKey)).toEqual(chainKey);
            expect(BigInt(node.lastCheckpointHeaderNumber)).toEqual(checkpointHeightToRevertTo);
            expect(BigInt(node.lastAttestedHeaderNumber)).toEqual(checkpointHeightToRevertTo);
            expect(node.lastAttestedDigest).toEqual(checkpointDigestToRevertTo);
        });
    });
});
