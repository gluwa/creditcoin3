import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { forElapsedBlocks } from '../utils';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventInvulnerableRegistered()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = (global as any).CREDITCOIN_CREATE_SIGNER('random');

        // make sure this attestor is not registered as invulnerable
        const response = await graphQLQuery(
            `query { invulnerableRegistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, whoId, chainKey, blockNumber }}}`,
        );
        let foundMatch = false;
        for (const node of response.data.invulnerableRegistereds.nodes) {
            if (node.attestorId === attestor.address) {
                foundMatch = true;
            }
        }
        expect(foundMatch).toEqual(false);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when attestor is made invulnerable', () => {
        beforeAll(async () => {
            // NOTE: act and observe GraphQL responses below
            await api.tx.sudo
                .sudo(api.tx.attestation.registerInvulnerable(chain_Anvil2_Key, attestor.address))
                .signAndSend(root);

            // wait for txn to make it on chain & indexer to ingest the block
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known InvulnerableRegistered entity', async () => {
            const response = await graphQLQuery(
                `query { invulnerableRegistereds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, whoId, chainKey, blockNumber } } }`,
            );
            expect(response.data.invulnerableRegistereds.nodes).toBeTruthy();
            expect(response.data.invulnerableRegistereds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.invulnerableRegistereds.nodes) {
                expect([chain_Anvil1_Key, chain_Anvil2_Key]).toContain(node.chainKey);
                expect(node.blockNumber).toBeGreaterThan(0);
                expect(node.attestorId).toBeTruthy();
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.whoId).toEqual(root.address);
                }

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { invulnerableRegistered(id: "${node.id}") { id, attestorId, whoId, chainKey, blockNumber } }`,
                );
                expect(response2.data.invulnerableRegistered).toBeTruthy();
                expect(response2.data.invulnerableRegistered.id).toEqual(node.id);
                expect(response2.data.invulnerableRegistered.attestorId).toEqual(node.attestorId);
                expect(response2.data.invulnerableRegistered.whoId).toEqual(node.whoId);
                expect(response2.data.invulnerableRegistered.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
