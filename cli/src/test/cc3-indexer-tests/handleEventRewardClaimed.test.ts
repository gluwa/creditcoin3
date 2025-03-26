import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventRewardClaimed()', () => {
    let api: ApiPromise;
    let alice: KeyringPair;
    let expectedReward: bigint;
    let startingBlock: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // alice has accumulated some rewards already
        const accumulatedRewards = await api.query.attestation.accumulatedRewards(alice.address);
        expect(accumulatedRewards.isSome).toBeTruthy();
        expectedReward = BigInt(accumulatedRewards.unwrap().toString());
        expect(expectedReward).toBeGreaterThan(0);

        startingBlock = (await getChainStatus(api)).bestNumber;

        let foundMatch = false;
        const response = await graphQLQuery(
            `query { rewardClaimeds (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
        );
        for (const node of response.data.rewardClaimeds.nodes) {
            if (node.stashId === alice.address && node.blockNumber >= startingBlock) {
                foundMatch = true;
            }
        }
        // alice hasn't claimed any rewards until current block
        expect(foundMatch).toEqual(false);
    }, 45_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when reward is claimed', () => {
        beforeAll(async () => {
            await api.tx.attestation.claimRewards().signAndSend(alice);
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known RewardClaimed entity', async () => {
            const response = await graphQLQuery(
                `query { rewardClaimeds (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
            );
            expect(response.data.rewardClaimeds.nodes).toBeTruthy();
            expect(response.data.rewardClaimeds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.rewardClaimeds.nodes) {
                expect(BigInt(node.amount)).toBeGreaterThan(0);
                expect(node.stashId).toBeTruthy();
                expect(node.whoId).toBeTruthy();
                expect(node.whoId).toEqual(node.stashId);
                // WARNING: cannot match attestorId b/c this value isn't recorded
                // best we can do is match stashId and look for record added in blocks
                // *AFTER* this test has started
                if (node.stashId === alice.address && node.blockNumber >= startingBlock) {
                    foundMatch = true;
                    expect(BigInt(node.amount)).toBeGreaterThanOrEqual(expectedReward);
                }
                // WARNING: ^^^ this is prone to false matches when we execute tests in parallel
                // and may fail to error out if there is a problem with indexer
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.blockNumber).toBeGreaterThan(0);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { rewardClaimed(id: "${node.id}") { id, amount, stashId, whoId, date, blockNumber } }`,
                );
                expect(response2.data.rewardClaimed).toBeTruthy();
                expect(response2.data.rewardClaimed.id).toEqual(node.id);
                expect(response2.data.rewardClaimed.amount).toEqual(node.amount);
                expect(response2.data.rewardClaimed.stashId).toEqual(node.stashId);
                expect(response2.data.rewardClaimed.whoId).toEqual(node.whoId);
                expect(response2.data.rewardClaimed.date).toEqual(node.date);
                expect(response2.data.rewardClaimed.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
