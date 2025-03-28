import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventAttestorElected()', () => {
    let api: ApiPromise;
    const activeAttestorsForAnvil1: string[] = [];

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when there are elected attestors', () => {
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

        it('graphQL returns known AttestorElected entity', async () => {
            let response = await graphQLQuery(
                `query { attestorsElecteds(orderBy: EPOCH_ASC, last: 1) { nodes { epoch }}}`,
            );
            // last record is for the current epoch or the next one in case it has changed meanwhile
            expect(BigInt(response.data.attestorsElecteds.nodes[0].epoch)).toBeGreaterThanOrEqual(currentEpoch);
            expect(BigInt(response.data.attestorsElecteds.nodes[0].epoch)).toBeLessThanOrEqual(currentEpoch + 1n);

            response = await graphQLQuery(
                `query {
                    attestorsElecteds(
                        orderBy: EPOCH_ASC,
                        last: 10,
                        filter: {
                            epoch: { equalTo: "${currentEpoch}"},
                            chainKey: { equalTo: ${chain_Anvil1_Key}},
                        }
                    ) { nodes { id, epoch, chainKey, attestorId }}
                }`,
            );
            expect(response.data.attestorsElecteds.nodes).toBeTruthy();
            expect(response.data.attestorsElecteds.nodes.length).toBeGreaterThanOrEqual(
                activeAttestorsForAnvil1.length,
            );

            for (const node of response.data.attestorsElecteds.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.epoch)).toEqual(currentEpoch);
                expect(node.chainKey).toEqual(chain_Anvil1_Key);
                expect(activeAttestorsForAnvil1).toContain(node.attestorId);

                const response2 = await graphQLQuery(
                    `query { attestorsElected(id: "${node.id}") { id, epoch, chainKey, attestorId }}`,
                );
                expect(response2.data.attestorsElected).toBeTruthy();
                expect(response2.data.attestorsElected.id).toEqual(node.id);
                expect(response2.data.attestorsElected.epoch).toEqual(node.epoch);
                expect(response2.data.attestorsElected.chainKey).toEqual(node.chainKey);
                expect(response2.data.attestorsElected.attestorId).toEqual(node.attestorId);
            }
        });

        it('graphQL returns updated Attestor entity', async () => {
            const response = await graphQLQuery(
                `query {
                    attestors(
                        orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC, last: 10,
                        filter: {
                            chainKey: { equalTo: ${chain_Anvil1_Key}},
                        }
                    ) { nodes { id, attestorId, lastUpdateBlockNumber, status }}
                }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            expect(response.data.attestors.nodes.length).toBeGreaterThanOrEqual(activeAttestorsForAnvil1.length);

            for (const node of response.data.attestors.nodes) {
                expect(activeAttestorsForAnvil1).toContain(node.attestorId);
                // attestor was last updated when it was elected
                expect(node.lastUpdateBlockNumber).toEqual(epochStart);
                expect(node.status).toEqual(3);
            }
        });
    });
});
