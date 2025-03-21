import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { randomFundedAccount, waitBlocks } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventBonded()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new attestor is registered', () => {
        beforeAll(async () => {
            // NOTE: registering the attestor will bond a fixed amount
            await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);
            await waitBlocks(3, api);
        }, 30_000);

        it('graphQL returns known Bonded entity', async () => {
            const response = await graphQLQuery(
                `query { bondeds (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
            );
            expect(response.data.bondeds.nodes).toBeTruthy();
            expect(response.data.bondeds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.bondeds.nodes) {
                expect(BigInt(node.amount)).toBeGreaterThan(0);
                expect(node.stashId).toBeTruthy();
                expect(node.whoId).toBeTruthy();
                // TODO: these are the same == bob
                expect(node.whoId).not.toEqual(node.stashId);
                // TODO: this will never match
                if (node.stashId === attestor.address) {
                    foundMatch = true;
                    expect(node.whoId).toEqual(bob.address);
                }
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.blockNumber).toBeGreaterThan(0);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { bonded(id: "${node.id}") { id, amount, stashId, whoId, date, blockNumber } }`,
                );
                expect(response2.data.bonded).toBeTruthy();
                expect(response2.data.bonded.id).toEqual(node.id);
                expect(response2.data.bonded.amount).toEqual(node.amount);
                expect(response2.data.bonded.stashId).toEqual(node.stashId);
                expect(response2.data.bonded.whoId).toEqual(node.whoId);
                expect(response2.data.bonded.date).toEqual(node.date);
                expect(response2.data.bonded.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toBeTruthy();
        });
    });
});
