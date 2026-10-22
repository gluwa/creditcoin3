import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil3_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { dispatchErrorName, forElapsedBlocks, sleep, sudoInBlock } from '../utils';
import { chainNow, graphQLQuery } from './common';

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
    let revertBlockHash: string;
    const chainKey = BigInt(chain_Anvil3_Key);

    // A `commit_attestation` signed by `root`, run through the pool's validation against the state
    // at `blockHash`. Whether it's admitted at all is decided before anything about its content or
    // signer is checked, so a dummy attestation from a non-attestor is enough. It's immortal and
    // uses `root`'s nonce at that block, so neither check rejects it at an older block.
    const commitValidityAt = async (blockHash: string) => {
        const at = await api.at(blockHash);
        const { nonce } = await at.query.system.account(root.address);
        const commit = api.tx.attestation.commitAttestation({
            attestation: {
                chainKey,
                headerNumber: 1,
                headerHash: new Uint8Array(32),
                root: new Uint8Array(32),
                prevDigest: null,
            },
            signature: new Uint8Array(96),
            attestors: [],
            continuityProof: { lowerEndpointDigest: new Uint8Array(32), roots: [] },
        });
        await commit.signAsync(root, { nonce, era: 0 });
        // Pass the encoded bytes, not hex: the metadata types this argument as Frontier's
        // self-contained extrinsic, which polkadot.js treats as `Bytes`, so a hex string would be
        // taken as raw content and length-prefixed a second time, and the runtime would panic
        // decoding it.
        return at.call.taggedTransactionQueue.validateTransaction('External', commit.toU8a(), blockHash);
    };

    const cleanupPendingAt = async (blockHash: string) => {
        const at = await api.at(blockHash);
        return {
            checkpointPruning: (await at.query.attestation.checkpointPruningStates(chainKey)).isSome,
            attestationClearing: (await at.query.attestation.attestationClearingCursors(chainKey)).isSome,
        };
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // A checkpoint every 2 attestations instead of the genesis 10, so the first one past
        // genesis comes within a couple of minutes of the attestors starting, however early
        // in the run this file is scheduled.
        await api.tx.sudo
            .sudo(api.tx.attestation.setAttestationsPerCheckpoint(chainKey, 2))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

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

            const revert = await sudoInBlock(
                api,
                root,
                api.tx.attestation.revertTo(chainKey, checkpointHeightToRevertTo),
            );
            expect(dispatchErrorName(api, revert.innerError)).toBeNull();
            revertBlockHash = revert.blockHash;

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
                expect(Date.parse(node.date)).toBeLessThanOrEqual(await chainNow(api));
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

            // The attestors keep attesting after the revert, and under `--sealing` they can
            // reach the next checkpoint before this runs. As for the rows above, the data may
            // only point past the reverted checkpoint at a checkpoint indexed after the revert.
            if (BigInt(node.lastCheckpointHeaderNumber) === checkpointHeightToRevertTo) {
                expect(BigInt(node.lastAttestedHeaderNumber)).toBeGreaterThanOrEqual(checkpointHeightToRevertTo);
                if (BigInt(node.lastAttestedHeaderNumber) === checkpointHeightToRevertTo) {
                    expect(node.lastAttestedDigest).toEqual(checkpointDigestToRevertTo);
                }
            } else {
                const checkpoint = await graphQLQuery(
                    `query {
                        checkpoints(
                            filter: {
                                chainKey: { equalTo: "${chainKey}" },
                                blockNumber: { equalTo: "${node.lastCheckpointHeaderNumber}" }
                            }
                        ) { nodes { atBlockNumber, timestamp } }
                    }`,
                );
                expect(checkpoint.data.checkpoints.nodes.length).toEqual(1);
                expect(BigInt(checkpoint.data.checkpoints.nodes[0].atBlockNumber)).toBeGreaterThan(blockBeforeRevert);
                expect(BigInt(checkpoint.data.checkpoints.nodes[0].timestamp)).toBeGreaterThan(timestampBeforeRevert);
            }
        });

        // A commit accepted while the revert's checkpoint pruning was still draining could have
        // its new checkpoint deleted along with the stale ones, so the pool must turn every commit
        // for the chain away until both cleanups have drained.
        describe('on chain', () => {
            let drainedBlockHash: string;

            it('keeps commits out of the pool while checkpoint pruning is pending', async () => {
                const pending = await cleanupPendingAt(revertBlockHash);
                expect(pending.checkpointPruning).toBe(true);

                const validity = await commitValidityAt(revertBlockHash);
                expect(validity.isErr).toBe(true);
                // `ATTESTATION_CLEANUP_IN_PROGRESS_CODE` in pallets/attestation/src/extensions.rs
                expect(validity.asErr.isInvalid && validity.asErr.asInvalid.isCustom).toBe(true);
                expect(validity.asErr.asInvalid.asCustom.toNumber()).toEqual(2);
            });

            it('lets commits back in once the cleanup has drained', async () => {
                const deadline = Date.now() + 120_000;
                for (;;) {
                    const hash = (await api.rpc.chain.getHeader()).hash.toHex();
                    const pending = await cleanupPendingAt(hash);
                    if (!pending.checkpointPruning && !pending.attestationClearing) {
                        drainedBlockHash = hash;
                        break;
                    }
                    if (Date.now() > deadline) {
                        throw new Error(`cleanup still pending at ${hash}: ${JSON.stringify(pending)}`);
                    }
                    await forElapsedBlocks(api, { minBlocks: 1 });
                }

                // `root` isn't an attestor, so the commit is admitted (and fails in dispatch).
                expect((await commitValidityAt(drainedBlockHash)).isOk).toBe(true);
            }, 150_000);

            it('keeps checkpoints the attestors reach after the cleanup', async () => {
                // Commits were blocked until the cleanup drained, so any checkpoint above the
                // reverted-to height was reached after it.
                const deadline = Date.now() + 600_000;
                let lastCheckpoint = (await api.query.attestation.lastCheckpoint(chainKey)).unwrap();
                while (lastCheckpoint.blockNumber.toBigInt() <= checkpointHeightToRevertTo) {
                    if (Date.now() > deadline) {
                        throw new Error('the attestors reached no new checkpoint after the revert');
                    }
                    await forElapsedBlocks(api, { minBlocks: 2 });
                    lastCheckpoint = (await api.query.attestation.lastCheckpoint(chainKey)).unwrap();
                }

                // Give any leftover pruning a few blocks to (wrongly) delete it.
                await forElapsedBlocks(api, { minBlocks: 3 });
                const stored = await api.query.attestation.checkpoints(chainKey, lastCheckpoint.blockNumber);
                expect(stored.isSome).toBe(true);
                expect(stored.unwrap().eq(lastCheckpoint.digest)).toBe(true);
            }, 660_000);
        });
    });
});
