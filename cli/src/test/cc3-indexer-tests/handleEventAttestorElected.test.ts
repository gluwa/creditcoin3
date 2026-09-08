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

        beforeAll(async () => {
            currentEpoch = (await api.query.babe.epochIndex()).toBigInt();
            expect(currentEpoch).toBeGreaterThan(0);

            // initial setup already has at least 3 attestors for Anvil 1
            const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
            for (const [_indx, account] of entriesForAnvil1) {
                activeAttestorsForAnvil1.push(account.toString());
            }
            // if they are already active this means they have been elected
            expect(activeAttestorsForAnvil1.length).toBeGreaterThan(0);
        }, 30_000);

        it('graphQL returns known AttestorElected entity', async () => {
            // `AttestorsElected` is emitted only when a chain's attestor set changes, so the most
            // recent election for Anvil 1 may be several epochs old. It must not be in the future
            // (allowing for an epoch rollover between reading babe and querying the indexer).
            const response0 = await graphQLQuery(
                `query {
                    attestorsElecteds(
                        orderBy: EPOCH_DESC,
                        first: 1,
                        filter: { chainKey: { equalTo: "${chain_Anvil1_Key}"} }
                    ) { nodes { epoch }}
                }`,
            );
            expect(response0.data.attestorsElecteds.nodes.length).toEqual(1);
            const latestElectionEpoch = BigInt(response0.data.attestorsElecteds.nodes[0].epoch);
            expect(latestElectionEpoch).toBeLessThanOrEqual(currentEpoch + 1n);

            // The latest election is what produced today's active set: every active attestor
            // must appear in it.
            const response = await graphQLQuery(
                `query {
                    attestorsElecteds(
                        orderBy: EPOCH_ASC,
                        last: 10,
                        filter: {
                            epoch: { equalTo: "${latestElectionEpoch}"},
                            chainKey: { equalTo: "${chain_Anvil1_Key}"},
                        }
                    ) { nodes { id, epoch, chainKey, attestorId }}
                }`,
            );
            expect(response.data.attestorsElecteds.nodes).toBeTruthy();
            const electedIds = response.data.attestorsElecteds.nodes.map(
                (node: { attestorId: string }) => node.attestorId,
            );
            for (const active of activeAttestorsForAnvil1) {
                expect(electedIds).toContain(active);
            }

            for (const node of response.data.attestorsElecteds.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.epoch)).toEqual(latestElectionEpoch);
                expect(node.chainKey).toEqual(chain_Anvil1_Key.toString());

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
                            chainKey: { equalTo: "${chain_Anvil1_Key}"},
                            status: { equalTo: 0 },
                        }
                    ) { nodes { id, attestorId, lastUpdateBlockNumber, status }}
                }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            expect(response.data.attestors.nodes.length).toBeGreaterThanOrEqual(activeAttestorsForAnvil1.length);

            const bestNumber = (await api.rpc.chain.getHeader()).number.toBigInt();
            for (const node of response.data.attestors.nodes) {
                expect(activeAttestorsForAnvil1).toContain(node.attestorId);
                // attestor was last updated when it was elected; elections are no longer repeated
                // every epoch, so that block can predate the current epoch by any amount
                expect(BigInt(node.lastUpdateBlockNumber)).toBeGreaterThan(0n);
                expect(BigInt(node.lastUpdateBlockNumber)).toBeLessThanOrEqual(bestNumber);
                expect(node.status).toEqual(0); // Active
            }
        });
    });
});
