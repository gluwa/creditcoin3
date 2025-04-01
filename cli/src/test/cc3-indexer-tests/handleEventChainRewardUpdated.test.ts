import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventChainRewardUpdated()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: number;
    const newRewardAmount = BigInt(randomIntBetween(500, 1000));

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new chain reward is set', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            // NOTE: by defauilt it is 1000
            await api.tx.sudo
                .sudo(api.tx.attestation.setChainReward(chain_Anvil2_Key, newRewardAmount))
                .signAndSend(root);
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known ChainRewardUpdated entity', async () => {
            const response = await graphQLQuery(
                `query { chainRewardUpdateds(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, whoId, amount, chainKey }}}`,
            );
            expect(response.data.chainRewardUpdateds.nodes).toBeTruthy();
            expect(response.data.chainRewardUpdateds.nodes.length).toEqual(1);

            for (const node of response.data.chainRewardUpdateds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.blockNumber).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.whoId).toEqual(root.address);
                expect(BigInt(node.amount)).toEqual(newRewardAmount);
                expect(node.chainKey).toEqual(chain_Anvil2_Key);
            }
        });

        it('graphQL returns updated AttestationChainData entity', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(
                        orderBy: CHAIN_KEY_ASC,
                        last: 1,
                        filter: { chainKey: { equalTo: ${chain_Anvil2_Key} }},
                    ) {
                        nodes { id, chainReward }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(BigInt(node.chainReward)).toEqual(newRewardAmount);
            }
        });
    });
});
