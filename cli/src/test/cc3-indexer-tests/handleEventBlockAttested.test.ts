import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key, chain_Anvil3_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventBlockAttested()', () => {
    let api: ApiPromise;
    const activeAttestorsForAnvil1: string[] = [];

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when there are attested blocks', () => {
        let currentEpoch = 0n;
        let epochStart = 0;
        let attestationInterval = 0;

        beforeAll(async () => {
            currentEpoch = (await api.query.babe.epochIndex()).toBigInt();
            expect(currentEpoch).toBeGreaterThan(0);

            epochStart = (await api.query.babe.epochStart())[1].toNumber();
            expect(epochStart).toBeGreaterThan(0);

            attestationInterval = (await api.query.attestation.chainAttestationInterval(chain_Anvil1_Key)).toNumber();
            expect(attestationInterval).toBeGreaterThan(0);

            // initial setup already has at least 3 attestors for Anvil 1
            const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
            for (const [_indx, account] of entriesForAnvil1) {
                activeAttestorsForAnvil1.push(account.toString());
            }
            // if they are already active this means they have been elected
            expect(activeAttestorsForAnvil1.length).toBeGreaterThan(0);
        }, 30_000);

        it('graphQL returns Attestations entities and keeps MapAttestationAttestor & AttestationChainData in sync', async () => {
            const response = await graphQLQuery(
                `query {
                    attestations(
                        orderBy: HEADER_NUMBER_ASC, last: 10,
                        filter: { chainKey: { equalTo: "${chain_Anvil1_Key}" }}
                    ) {
                        nodes { id, chainKey, headerNumber, headerHash, root, prevDigest, signature, digest, timestamp, continuityProof }
                    },

                    attestationChainData(
                        orderBy: CHAIN_KEY_ASC,
                        last: 1,
                        filter: { chainKey: { equalTo: "${chain_Anvil1_Key}" }}
                    ) { nodes { id, lastAttestedDigest, lastAttestedHeaderNumber }},
                }`,
            );
            expect(response.data.attestations.nodes).toBeTruthy();
            expect(response.data.attestations.nodes.length).toBeGreaterThan(0);

            let lastHeaderNumber = 0n;
            let lastDigest = '';
            for (const node of response.data.attestations.nodes) {
                expect(node.id).toBeTruthy();
                expect([chain_Anvil1_Key.toString(), chain_Anvil3_Key.toString()]).toContain(node.chainKey);
                expect(BigInt(node.headerNumber)).toBeGreaterThanOrEqual(0n);
                lastHeaderNumber = BigInt(node.headerNumber);

                expect(node.headerHash.startsWith('0x')).toEqual(true);
                // next 2 fields are essentially empty
                expect(node.root).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');

                expect(node.signature.startsWith('0x')).toEqual(true);
                expect(node.digest.startsWith('0x')).toEqual(true);
                if (lastHeaderNumber === 0n) {
                    expect(node.prevDigest).toEqual('');
                } else {
                    expect(node.prevDigest.startsWith('0x')).toEqual(true);
                }

                // 0 means that the block timestamp wasn't present, and it defaulted to 0, which is a problem
                expect(BigInt(node.timestamp)).toBeGreaterThan(0);
                expect(BigInt(node.timestamp)).toBeLessThan(Date.now());
                lastDigest = node.digest;

                // for each Attestation entity there is at least 1 entry in MapAttestationAttestor
                const mapResponse = await graphQLQuery(
                    `query {
                        mapAttestationAttestors(
                            orderBy: ID_ASC,
                            last: 10,
                            filter: {
                                attestationId: { equalTo: "${node.id}" }
                            }
                        ) { nodes { id, attestorId, attestationId }}
                    }`,
                );
                expect(mapResponse.data.mapAttestationAttestors.nodes.length).toBeGreaterThanOrEqual(1);
                // ^^^ this is just a many-to-many mapping table. Not sure how to assert that
                // the relationships are the correct ones!
                if (node.headerNumber > 0) {
                    // continuityProof is only present for attestations after the first one
                    // and should contain at least attestationInterval - 1 blocks
                    expect(node.continuityProof).toBeTruthy();
                    expect(node.continuityProof.blocks).toBeTruthy();
                    expect(node.continuityProof.blocks.length).toBeGreaterThanOrEqual(attestationInterval - 1);
                } else {
                    expect(node.continuityProof.blocks.length).toEqual(0);
                }
            }
            expect(lastHeaderNumber).toBeGreaterThan(0n);
            expect(lastDigest).not.toEqual('');

            // chain data records were also updated
            expect(response.data.attestationChainData.nodes[0].lastAttestedDigest).toEqual(lastDigest);
            expect(BigInt(response.data.attestationChainData.nodes[0].lastAttestedHeaderNumber)).toEqual(
                lastHeaderNumber,
            );
        });
    });
});
