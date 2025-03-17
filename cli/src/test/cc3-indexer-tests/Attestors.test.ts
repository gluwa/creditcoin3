import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('Attestors', () => {
    let api: ApiPromise;
    const attestorsForAnvil1: string[] = [];
    const attestorsForAnvil2: string[] = [];

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        // NOTE: initial setup already has at least 3 attestors for each of source chain 2 & 4
        // and by the time this executes they are probably already actively attesting!
        // Not going to register new ones here
        const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
        for (const [_indx, account] of entriesForAnvil1) {
            attestorsForAnvil1.push(account.toString());
        }

        const entriesForAnvil2 = (await api.query.attestation.activeAttestors(chain_Anvil2_Key)).entries();
        for (const [_indx, account] of entriesForAnvil2) {
            attestorsForAnvil2.push(account.toString());
        }
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('returns known AttestorRegistered', async () => {
        const response = await graphQLQuery(
            `query { attestorRegistereds(last: 10) { nodes { id, attestorId, stashId, chainKey, blockNumber } } }`,
        );
        expect(response.data.attestorRegistereds.nodes).toBeTruthy();
        expect(response.data.attestorRegistereds.nodes.length).toEqual(
            attestorsForAnvil1.length + attestorsForAnvil2.length,
        );

        for (const node of response.data.attestorRegistereds.nodes) {
            expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
            expect(node.blockNumber).toBeGreaterThan(0);
            expect(node.attestorId).toBeTruthy();
            // match what's registered on-chain
            if (node.chainKey === chain_Anvil1_Key) {
                expect(attestorsForAnvil1).toContain(node.attestorId);
            } else {
                expect(attestorsForAnvil2).toContain(node.attestorId);
            }
            expect(node.stashId).toBeTruthy();
            expect(node.attestorId).not.toEqual(node.stashId);

            // query each node individually to cover this endpoint too
            const response2 = await graphQLQuery(
                `query { attestorRegistered(id: "${node.id}") { id, attestorId, stashId, chainKey, blockNumber } }`,
            );
            expect(response2.data.attestorRegistered).toBeTruthy();
            expect(response2.data.attestorRegistered.id).toEqual(node.id);
            expect(response2.data.attestorRegistered.attestorId).toEqual(node.attestorId);
            expect(response2.data.attestorRegistered.stashId).toEqual(node.stashId);
            expect(response2.data.attestorRegistered.blockNumber).toEqual(node.blockNumber);
        }
    });

    it('returns known Attestor', async () => {
        const response = await graphQLQuery(
            `query { attestors(last: 10) { nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } } }`,
        );
        expect(response.data.attestors.nodes).toBeTruthy();
        expect(response.data.attestors.nodes.length).toEqual(attestorsForAnvil1.length + attestorsForAnvil2.length);

        for (const node of response.data.attestors.nodes) {
            expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
            expect(node.lastUpdateBlockNumber).toBeGreaterThan(0);
            expect(node.status).toBeGreaterThan(0);
            expect(node.blsPublicKey).toBeTruthy();
            expect(node.attestorId).toBeTruthy();
            // match what's registered on-chain
            if (node.chainKey === chain_Anvil1_Key) {
                expect(attestorsForAnvil1).toContain(node.attestorId);
            } else {
                expect(attestorsForAnvil2).toContain(node.attestorId);
            }
            expect(node.stashId).toBeTruthy();
            expect(node.attestorId).not.toEqual(node.stashId);

            // query each node individually to cover this endpoint too
            const response2 = await graphQLQuery(
                `query { attestor(id: "${node.id}") { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } }`,
            );
            expect(response2.data.attestor).toBeTruthy();
            expect(response2.data.attestor.id).toEqual(node.id);
            expect(response2.data.attestor.attestorId).toEqual(node.attestorId);
            expect(response2.data.attestor.stashId).toEqual(node.stashId);
            expect(response2.data.attestor.blsPublicKey).toEqual(node.blsPublicKey);
            expect(response2.data.attestor.lastUpdateBlockNumber).toEqual(node.lastUpdateBlockNumber);
        }
    });
});
