import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventAttestorUnregistered()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // NOTE: Bob is the STASH for a random attestor on the Anvil2 chain
        await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);

        // wait for txn to make it on chain so we can deregister later
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 45_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new attestor is unregistered', () => {
        beforeAll(async () => {
            // make sure attestor is reported as registered
            let response = await graphQLQuery(
                `query { attestors(orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } } }`,
            );
            let foundMatch = false;
            for (const node of response.data.attestors.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.stashId).toEqual(bob.address);
                    expect(node.status).toEqual(1); // has just been registered
                }
            }
            expect(foundMatch).toEqual(true);

            // make sure this attestor is not reported as previously unregistered
            response = await graphQLQuery(
                `query { attestorUnregistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, chainKey, blockNumber }}}`,
            );
            foundMatch = false;
            for (const node of response.data.attestorUnregistereds.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(false);

            // NOTE: now remove it and observe GraphQL responses below
            await api.tx.attestation.unregisterAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);

            // wait for txn to make it on chain & indexer to ingest the block
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known AttestorUnregistered entity', async () => {
            const response = await graphQLQuery(
                `query { attestorUnregistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, chainKey, blockNumber } } }`,
            );
            expect(response.data.attestorUnregistereds.nodes).toBeTruthy();
            expect(response.data.attestorUnregistereds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.attestorUnregistereds.nodes) {
                expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
                expect(node.blockNumber).toBeGreaterThan(0);
                expect(node.attestorId).toBeTruthy();
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                }

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { attestorUnregistered(id: "${node.id}") { id, attestorId, chainKey, blockNumber } }`,
                );
                expect(response2.data.attestorUnregistered).toBeTruthy();
                expect(response2.data.attestorUnregistered.id).toEqual(node.id);
                expect(response2.data.attestorUnregistered.attestorId).toEqual(node.attestorId);
                expect(response2.data.attestorUnregistered.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });

        it('graphQL returns known Attestor entity', async () => {
            const response = await graphQLQuery(
                `query { attestors(orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } } }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            expect(response.data.attestors.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.attestors.nodes) {
                expect(node.lastUpdateBlockNumber).toBeGreaterThan(0);

                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.status).toEqual(2); // was 1 before
                }
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
