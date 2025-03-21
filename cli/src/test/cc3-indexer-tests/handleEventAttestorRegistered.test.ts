import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventAttestorRegistered()', () => {
    let api: ApiPromise;
    const activeAttestorsForAnvil1: string[] = [];
    const activeAttestorsForAnvil2: string[] = [];
    let bob: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // NOTE: initial setup already has at least 3 attestors for each of source chain 2 & 4
        // and by the time this executes they are probably already actively attesting!
        // Not going to register new ones here
        const entriesForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).entries();
        for (const [_indx, account] of entriesForAnvil1) {
            activeAttestorsForAnvil1.push(account.toString());
        }

        const entriesForAnvil2 = (await api.query.attestation.activeAttestors(chain_Anvil2_Key)).entries();
        for (const [_indx, account] of entriesForAnvil2) {
            activeAttestorsForAnvil2.push(account.toString());
        }

        // attestor hasn't been registered yet!
        expect(activeAttestorsForAnvil2).not.toContain(attestor.address);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new attestor is registered', () => {
        beforeAll(async () => {
            // NOTE: Bob is the STASH for a random attestor on the Anvil2 chain
            await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);

            // wait for txn to make it on chain & indexer to ingest the block
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known AttestorRegistered', async () => {
            // note: ^^^ there could be entries for attestors which registered but
            // did not become active for example
            const response = await graphQLQuery(
                `query { attestorRegistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, stashId, chainKey, blockNumber } } }`,
            );
            expect(response.data.attestorRegistereds.nodes).toBeTruthy();
            // initial + current + other
            expect(response.data.attestorRegistereds.nodes.length).toBeGreaterThanOrEqual(
                activeAttestorsForAnvil1.length + activeAttestorsForAnvil2.length,
            );

            let foundMatch = false;
            for (const node of response.data.attestorRegistereds.nodes) {
                expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
                expect(node.blockNumber).toBeGreaterThan(0);
                expect(node.attestorId).toBeTruthy();
                // match what's registered on-chain
                if (node.chainKey === chain_Anvil1_Key) {
                    expect(activeAttestorsForAnvil1).toContain(node.attestorId);
                }
                expect(node.stashId).toBeTruthy();
                expect(node.attestorId).not.toEqual(node.stashId);
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.stashId).toEqual(bob.address);
                }

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
            expect(foundMatch).toBeTruthy();
        });

        it('graphQL returns known Attestor', async () => {
            const response = await graphQLQuery(
                `query { attestors(orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } } }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            // initial + other
            expect(response.data.attestors.nodes.length).toBeGreaterThanOrEqual(
                activeAttestorsForAnvil1.length + activeAttestorsForAnvil2.length,
            );

            let foundMatch = false;
            for (const node of response.data.attestors.nodes) {
                expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
                expect(node.lastUpdateBlockNumber).toBeGreaterThan(0);
                expect(node.status).toBeGreaterThan(0);
                expect(node.attestorId).toBeTruthy();
                // match what's registered on-chain
                if (node.chainKey === chain_Anvil1_Key) {
                    expect(activeAttestorsForAnvil1).toContain(node.attestorId);
                }
                expect(node.stashId).toBeTruthy();
                expect(node.attestorId).not.toEqual(node.stashId);

                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.stashId).toEqual(bob.address);
                    expect(node.status).toEqual(1);
                }

                // only active attestors have their blsPublicKey set
                if (node.status === 3) {
                    expect(node.blsPublicKey).toBeTruthy();
                } else {
                    expect(node.blsPublicKey).toBeFalsy();
                }

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
            expect(foundMatch).toBeTruthy();
        });
    });
});
