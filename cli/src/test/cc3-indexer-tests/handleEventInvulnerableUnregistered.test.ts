import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { forElapsedBlocks } from '../utils';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventInvulnerableUnregistered()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = (global as any).CREDITCOIN_CREATE_SIGNER('random');

        // register as invulnerable just so we can remove it later
        await api.tx.sudo
            .sudo(api.tx.attestation.registerInvulnerable(chain_Anvil2_Key, attestor.address))
            .signAndSend(root);
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when an invulnerable is removed', () => {
        beforeAll(async () => {
            // make sure attestor is reported as invulnerable
            let response = await graphQLQuery(
                `query { invulnerableRegistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, whoId, chainKey, blockNumber }}}`,
            );
            let foundMatch = false;
            for (const node of response.data.invulnerableRegistereds.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.whoId).toEqual(root.address);
                }
            }
            expect(foundMatch).toEqual(true);

            // make sure invulnerable is not reported as previously unregistered
            response = await graphQLQuery(
                `query { invulnerableUnregistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, whoId, chainKey, blockNumber }}}`,
            );
            foundMatch = false;
            for (const node of response.data.invulnerableUnregistereds.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(false);

            // act and observe GraphQL responses below
            await api.tx.sudo
                .sudo(api.tx.attestation.unregisterInvulnerable(chain_Anvil2_Key, attestor.address))
                .signAndSend(root);
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known InvulnerableUnregistered entity', async () => {
            const response = await graphQLQuery(
                `query { invulnerableUnregistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, whoId, chainKey, blockNumber } } }`,
            );
            expect(response.data.invulnerableUnregistereds.nodes).toBeTruthy();
            expect(response.data.invulnerableUnregistereds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.invulnerableUnregistereds.nodes) {
                expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
                expect(node.blockNumber).toBeGreaterThan(0);
                expect(node.attestorId).toBeTruthy();
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.whoId).toEqual(root.address);
                }

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { invulnerableUnregistered(id: "${node.id}") { id, attestorId, whoId, chainKey, blockNumber } }`,
                );
                expect(response2.data.invulnerableUnregistered).toBeTruthy();
                expect(response2.data.invulnerableUnregistered.id).toEqual(node.id);
                expect(response2.data.invulnerableUnregistered.attestorId).toEqual(node.attestorId);
                expect(response2.data.invulnerableUnregistered.whoId).toEqual(node.whoId);
                expect(response2.data.invulnerableUnregistered.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
