import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil3_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventRevertedAttestationChainTo()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;

    let checkpointHeightToRevertTo: bigint;
    let checkpointDigestToRevertTo: string;
    let laterCheckpointHeight: bigint;
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

        // Save to check for removal later
        laterCheckpointHeight = BigInt(checkpoints[1].blockNumber);
    }, 2_000_000); // Need timeout long enough to generate first non-genesis checkpoint

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when the attestation chain is reverted to a checkpoint', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.attestation.revertTo(chainKey, checkpointHeightToRevertTo))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            await forElapsedBlocks(api, { minBlocks: 5, maxRetries: 15 });
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
                        }
                    }
                }`,
            );

            expect(response.data.revertedAttestationChainTos.nodes).toBeTruthy();
            expect(response.data.revertedAttestationChainTos.nodes.length).toEqual(1);

            for (const node of response.data.revertedAttestationChainTos.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.blockNumber)).toBeGreaterThanOrEqual(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.chainKey).toEqual(chainKey.toString());
                expect(BigInt(node.checkpointHeight)).toEqual(checkpointHeightToRevertTo);
                expect(node.digest).toEqual(checkpointDigestToRevertTo);
            }
        });

        it('removes checkpoints above checkpointHeight', async () => {
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

            expect(response.data.checkpoints.nodes).toBeTruthy();
            // Assert the specific pre-revert checkpoint that existed above the revert
            // height was removed. We intentionally do NOT assert that *every* remaining
            // checkpoint is <= checkpointHeight: the chain keeps attesting after the
            // revert, so new legitimate checkpoints above the revert height are indexed
            // shortly afterwards and would race this assertion.
            expect(
                response.data.checkpoints.nodes.some(
                    (node: { blockNumber: string }) => BigInt(node.blockNumber) === laterCheckpointHeight,
                ),
            ).toEqual(false);
        });

        it('removes attestations above checkpointHeight', async () => {
            const response = await graphQLQuery(
                `query {
                    attestations(
                        filter: { chainKey: { equalTo: "${chainKey}" }},
                        orderBy: HEADER_NUMBER_ASC
                    ) {
                        nodes {
                            id
                            headerNumber
                            digest
                        }
                    }
                }`,
            );

            expect(response.data.attestations.nodes).toBeTruthy();
            // Assert the specific attestation that existed above the revert height
            // (at `laterCheckpointHeight`) was removed. We intentionally do NOT assert
            // that every remaining attestation is <= checkpointHeight: post-revert the
            // chain keeps attesting, so new legitimate attestations above the revert
            // height get indexed and would race a blanket assertion.
            expect(
                response.data.attestations.nodes.some(
                    (node: { headerNumber: string }) => BigInt(node.headerNumber) === laterCheckpointHeight,
                ),
            ).toEqual(false);
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
