import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventUnbonded()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let attestor: any;
    let startingBlock: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // register here just so we can unregister a bit later
        await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 45_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new attestor is unregistered', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;

            // NOTE: unregistering the attestor will also unbond
            await api.tx.attestation.unregisterAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known Unbonded entity', async () => {
            const response = await graphQLQuery(
                `query { unbondeds (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
            );
            expect(response.data.unbondeds.nodes).toBeTruthy();
            expect(response.data.unbondeds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.unbondeds.nodes) {
                expect(BigInt(node.amount)).toBeGreaterThan(0);
                expect(node.stashId).toBeTruthy();
                expect(node.whoId).toBeTruthy();
                expect(node.whoId).toEqual(node.stashId);
                // WARNING: cannot match attestorId b/c this value isn't recorded
                // best we can do is match stashId and look for record added in blocks
                // *AFTER* this test has started
                if (node.stashId === bob.address && node.blockNumber > startingBlock) {
                    foundMatch = true;
                }
                // WARNING: ^^^ this is prone to false matches when we execute tests in parallel
                // and may fail to error out if there is a problem with indexer
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.blockNumber).toBeGreaterThan(0);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { unbonded(id: "${node.id}") { id, amount, stashId, whoId, date, blockNumber } }`,
                );
                expect(response2.data.unbonded).toBeTruthy();
                expect(response2.data.unbonded.id).toEqual(node.id);
                expect(response2.data.unbonded.amount).toEqual(node.amount);
                expect(response2.data.unbonded.stashId).toEqual(node.stashId);
                expect(response2.data.unbonded.whoId).toEqual(node.whoId);
                expect(response2.data.unbonded.date).toEqual(node.date);
                expect(response2.data.unbonded.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toBeTruthy();
        });
    });
});
