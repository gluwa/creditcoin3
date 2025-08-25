import { hexToString } from '@polkadot/util';
import { Option, U32, U64, U128 } from '@polkadot/types-codec';
import { SupportedChainsPrimitivesSupportedChain } from '@polkadot/types/lookup';
import { newApi, ApiPromise } from '../../lib';
import { graphQLQuery } from './common';

describe('initiateStoreAndDatabase()', () => {
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when indexer is running', () => {
        it('graphQL returns AttestationChainData which matches the Creditcoin3 chain storage', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(orderBy: CHAIN_KEY_ASC, last: 10) {
                        nodes {
                            id,
                            chainKey,
                            attestationInterval,
                            checkpointInterval,
                            chainReward,
                            maxSetSize,
                            targetSampleSize,
                            minBondRequirement,
                            voteAcceptanceWindow
                        }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes).toBeTruthy();
            expect(response.data.attestationChainData.nodes.length).toBeGreaterThan(0);

            for (const node of response.data.attestationChainData.nodes) {
                // such source chain exists
                const sourceChain = (
                    (await api.query.supportedChains.supportedChains(
                        node.chainKey,
                    )) as Option<SupportedChainsPrimitivesSupportedChain>
                ).unwrap();
                expect(sourceChain.chainId.toBigInt()).toBeGreaterThan(0n);

                const attestationInterval = (
                    (await api.query.attestation.chainAttestationInterval(node.chainKey)) as U64
                ).toBigInt();
                expect(BigInt(node.attestationInterval)).toEqual(attestationInterval);

                const checkpointInterval = (
                    (await api.query.attestation.attestationCheckpointInterval(node.chainKey)) as U32
                ).toNumber();
                expect(node.checkpointInterval).toEqual(checkpointInterval);

                const chainReward = ((await api.query.attestation.chainReward(node.chainKey)) as Option<U128>)
                    .unwrap()
                    .toBigInt();
                expect(BigInt(node.chainReward)).toEqual(chainReward);

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
            }
        }, 15_000);

        it('graphQL returns SupportedChains which match the Creditcoin3 chain storage', async () => {
            const response = await graphQLQuery(
                `query {
                    supportedChains(
                        orderBy: CHAIN_KEY_ASC,
                    ) { nodes { id, chainKey, chainName, chainId }}}`,
            );
            expect(response.data.supportedChains.nodes).toBeTruthy();
            // starting with 4 initial chain in Genesis but we'll inspect all currently supported
            expect(response.data.supportedChains.nodes.length).toBeGreaterThanOrEqual(4);

            for (const node of response.data.supportedChains.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.chainKey)).toBeGreaterThan(0n);

                // such source exists in on-chain storage
                const sourceChain = (
                    (await api.query.supportedChains.supportedChains(
                        node.chainKey,
                    )) as Option<SupportedChainsPrimitivesSupportedChain>
                ).unwrap();
                // GraphQL & on-chain values match
                expect(BigInt(node.chainId)).toEqual(sourceChain.chainId.toBigInt());
                expect(node.chainName).toEqual(hexToString(sourceChain.chainName.toString()));
            }
        }, 15_000);
    });
});
