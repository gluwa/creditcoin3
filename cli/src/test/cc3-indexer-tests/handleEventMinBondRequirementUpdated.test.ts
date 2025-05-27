import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';

describe('handleEventMinBondRequirementUpdated()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: number;
    const newMinBondAmount = BigInt(randomIntBetween(50, 100));

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when min bond amount is updated', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            // NOTE: by defauilt it is 100
            await api.tx.sudo
                .sudo(api.tx.attestation.setMinBondRequirement(chain_Anvil1_Key, newMinBondAmount))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known MinBondRequirementUpdated entity', async () => {
            const response = await graphQLQuery(
                `query { minBondRequirementUpdateds(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, whoId, amount, chainKey }}}`,
            );
            expect(response.data.minBondRequirementUpdateds.nodes).toBeTruthy();
            expect(response.data.minBondRequirementUpdateds.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.minBondRequirementUpdateds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.blockNumber).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.whoId).toEqual(root.address);
                expect(BigInt(node.amount)).toEqual(newMinBondAmount);
                expect(node.chainKey).toEqual(chain_Anvil1_Key.toString());
            }
        });

        it('graphQL returns updated AttestationChainData entity', async () => {
            const response = await graphQLQuery(
                `query { attestationChainData(orderBy: CHAIN_KEY_ASC, last: 100, filter: {chainKey: {equalTo: "${chain_Anvil1_Key}"}}) { nodes { id, minBondRequirement }}}`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(BigInt(node.minBondRequirement)).toEqual(newMinBondAmount);
            }
        });
    });
});
