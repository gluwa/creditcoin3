import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventRewardPaid()', () => {
    let api: ApiPromise;
    let alice: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // alice has accumulated some rewards already
        const accumulatedRewards = await api.query.attestation.accumulatedRewards(alice.address);
        expect(accumulatedRewards.isSome).toBeTruthy();
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when there are accumulated rewards', () => {
        it('graphQL returns known RewardPaid entities', async () => {
            const response = await graphQLQuery(
                `query { rewardPaids (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, chainKey, date, blockNumber }}}`,
            );
            expect(response.data.rewardPaids.nodes).toBeTruthy();
            expect(response.data.rewardPaids.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.rewardPaids.nodes) {
                // we don't have active attestors for Anvil 2
                expect(node.chainKey).toEqual(chain_Anvil1_Key);
                expect(BigInt(node.amount)).toBeGreaterThan(0);
                expect(node.stashId).toBeTruthy();
                // only Alice has active attestors
                expect(node.stashId).toEqual(alice.address);
                expect(node.whoId).toBeTruthy();
                // commit_attestation() contains ensure_none(origin)?;
                // whoId is Origin::none()
                expect(node.stashId).not.toEqual(node.whoId);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.blockNumber).toBeGreaterThan(0);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { rewardPaid(id: "${node.id}") { id, amount, stashId, whoId, chainKey, date, blockNumber } }`,
                );
                expect(response2.data.rewardPaid).toBeTruthy();
                expect(response2.data.rewardPaid.id).toEqual(node.id);
                expect(response2.data.rewardPaid.amount).toEqual(node.amount);
                expect(response2.data.rewardPaid.stashId).toEqual(node.stashId);
                expect(response2.data.rewardPaid.whoId).toEqual(node.whoId);
                expect(response2.data.rewardPaid.chainKey).toEqual(node.chainKey);
                expect(response2.data.rewardPaid.date).toEqual(node.date);
                expect(response2.data.rewardPaid.blockNumber).toEqual(node.blockNumber);
            }
        });
    });
});
