import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil3_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventCheckpointsCleared()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        let hasCheckpoints = false;
        while (!hasCheckpoints) {
            await forElapsedBlocks(api, { minBlocks: 2 });
            const lastCheckpoint = await api.query.attestation.lastCheckpoint(chain_Anvil3_Key);

            if (lastCheckpoint.isSome) {
                const checkpointData = lastCheckpoint.unwrap();

                // Brad says you need 2 checkpoints worth of attestations before the first
                // checkpoint is actually created therefore wait a min of 200 blocks
                hasCheckpoints = checkpointData.blockNumber.toBigInt() > 200n;
            }
        }
    }, 1_500_000); // ~ 300 blocks

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when a supported chain is removed', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0);

            await api.tx.sudo
                .sudo(api.tx.supportedChains.removeChain(chain_Anvil3_Key, true))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known CheckpointsCleared entity', async () => {
            const response = await graphQLQuery(
                `query { checkpointsCleareds(
                    orderBy: BLOCK_NUMBER_ASC,
                    filter: { chainKey: { equalTo: "${chain_Anvil3_Key}" }},
                    last: 1,
                ) { nodes { id, blockNumber, date, chainKey }}}`,
            );
            expect(response.data.checkpointsCleareds.nodes).toBeTruthy();
            expect(response.data.checkpointsCleareds.nodes.length).toEqual(1);

            for (const node of response.data.checkpointsCleareds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.blockNumber)).toBeGreaterThanOrEqual(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.chainKey).toEqual(chain_Anvil3_Key.toString());
            }
        });
    });
});
