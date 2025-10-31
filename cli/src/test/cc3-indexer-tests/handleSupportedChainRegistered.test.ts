import { U32, U64, U128 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleSupportedChainRegistered()', () => {
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
    }, 30_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
    });

    describe('when a new chain is registered', () => {
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

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known ChainRegistered entity', async () => {
            const response = await graphQLQuery(
                `query {
                    chainRegistereds(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, at, chainKey, chainName, chainId, chainEncoding, maturityStrategy, whoId }}}`,
            );
            expect(response.data.chainRegistereds.nodes).toBeTruthy();
            expect(response.data.chainRegistereds.nodes.length).toEqual(1);

            for (const node of response.data.chainRegistereds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.at)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(node.chainName).toEqual(newChainName);
                expect(BigInt(node.chainId)).toEqual(BigInt(newChainId));
                expect(node.chainEncoding).toEqual(encoding);
                expect(node.maturityStrategy).toEqual(defaultMaturityStrategy);
                expect(node.whoId).toEqual(root.address);
            }
        });

        it('graphQL returns known SupportedChain entity', async () => {
            const response = await graphQLQuery(
                `query {
                    supportedChains(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) { nodes { id, chainKey, chainName, chainId, chainEncoding, maturityStrategy }}}`,
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
        });

        it('graphQL returns known AttestationChainData entity', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        last: 1,
                    ) {
                        nodes {
                            id,
                            chainKey,
                            attestationInterval,
                            checkpointInterval,
                            lastAttestedDigest,
                            lastAttestedHeaderNumber,
                            lastCheckpointHeaderNumber,
                            maxSetSize,
                            targetSampleSize,
                            minBondRequirement,
                            voteAcceptanceWindow,
                            electionPolicy
                        }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes).toBeTruthy();
            expect(response.data.attestationChainData.nodes.length).toEqual(1);

            for (const node of response.data.attestationChainData.nodes) {
                expect(node.id).toEqual(newChainKey.toString());
                expect(BigInt(node.chainKey)).toEqual(newChainKey);

                const attestationInterval = (
                    (await api.query.attestation.chainAttestationInterval(node.chainKey)) as U64
                ).toBigInt();
                expect(BigInt(node.attestationInterval)).toEqual(attestationInterval);

                const checkpointInterval = (
                    (await api.query.attestation.attestationCheckpointInterval(node.chainKey)) as U32
                ).toNumber();
                expect(node.checkpointInterval).toEqual(checkpointInterval);

                expect(node.lastAttestedDigest).toEqual('');
                expect(BigInt(node.lastAttestedHeaderNumber)).toEqual(0n);
                expect(BigInt(node.lastCheckpointHeaderNumber)).toEqual(0n);

                const maxAttestors = ((await api.query.attestation.maxAttestors(node.chainKey)) as U32).toNumber();
                expect(node.maxSetSize).toEqual(maxAttestors);

                const targetSampleSize = (
                    (await api.query.attestation.targetSampleSize(node.chainKey)) as U32
                ).toNumber();
                expect(node.targetSampleSize).toEqual(targetSampleSize);

                const minBondRequirement = (await api.query.attestation.minBondRequirement(node.chainKey)) as U128;
                expect(node.minBondRequirement).toEqual(minBondRequirement.toString());

                const voteAcceptanceWindow = (
                    (await api.query.attestation.voteAcceptanceWindow(node.chainKey)) as U64
                ).toBigInt();
                expect(BigInt(node.voteAcceptanceWindow)).toEqual(voteAcceptanceWindow);

                const electionPolicy = (await api.query.attestation.chainElectionPolicy(node.chainKey)).toString();
                expect(node.electionPolicy).toEqual(electionPolicy);
            }
        });
    });
});
