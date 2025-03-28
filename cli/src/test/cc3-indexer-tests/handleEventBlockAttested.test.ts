import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
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

        beforeAll(async () => {
            currentEpoch = (await api.query.babe.epochIndex()).toBigInt();
            expect(currentEpoch).toBeGreaterThan(0);

            epochStart = (await api.query.babe.epochStart())[1].toNumber();
            expect(epochStart).toBeGreaterThan(0);

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
                `query { attestations(orderBy: HEADER_NUMBER_ASC, last: 10) { nodes { id, chainKey, headerNumber, headerHash, root, prevDigest, signature, digest }}}`,
            );
            expect(response.data.attestations.nodes).toBeTruthy();
            expect(response.data.attestations.nodes.length).toBeGreaterThan(0);

            let lastHeaderNumber = 0;
            let lastDigest = '';
            for (const node of response.data.attestations.nodes) {
                expect(node.id).toBeTruthy();
                expect(node.chainKey).toEqual(chain_Anvil1_Key);
                expect(node.headerNumber).toBeGreaterThanOrEqual(0);
                lastHeaderNumber = node.headerNumber;

                expect(node.headerHash.startsWith('0x')).toEqual(true);
                // next 2 fields are essentially empty
                expect(node.root).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
                expect(node.prevDigest).toEqual('');

                expect(node.signature.startsWith('0x')).toEqual(true);
                expect(node.digest.startsWith('0x')).toEqual(true);
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
            }
            expect(lastHeaderNumber).toBeGreaterThan(0);
            expect(lastDigest).not.toEqual('');

            // this was updated
            const chainDataResponse = await graphQLQuery(
                `query {
                    attestationChainData(
                        orderBy: CHAIN_KEY_ASC,
                        last: 1,
                        filter: { chainKey: { equalTo: ${chain_Anvil1_Key} }}
                    ) { nodes { id, lastAttestedDigest, lastAttestedHeaderNumber }}
                }`,
            );
            expect(chainDataResponse.data.attestationChainData.nodes[0].lastAttestedDigest).toEqual(lastDigest);
            expect(chainDataResponse.data.attestationChainData.nodes[0].lastAttestedHeaderNumber).toEqual(
                lastHeaderNumber,
            );
        });
    });
});
