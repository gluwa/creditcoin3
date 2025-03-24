import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventCheckpointReached()', () => {
    let api: ApiPromise;
    let lastBlockNumber: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        const lastCheckpoint = await api.query.attestation.lastCheckpoint(chain_Anvil1_Key);
        expect(lastCheckpoint.isSome).toBeTruthy();
        lastBlockNumber = lastCheckpoint.unwrap().blockNumber.toNumber();
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when there are checkpoints on chain', () => {
        it('graphQL returns known Checkpoint entities', async () => {
            const response = await graphQLQuery(
                `query { checkpoints(orderBy: AT_BLOCK_NUMBER_ASC, last: 10) { nodes { id, whoId, atBlockNumber, chainKey, blockNumber, digest }}}`,
            );
            expect(response.data.checkpoints.nodes).toBeTruthy();
            expect(response.data.checkpoints.nodes.length).toBeGreaterThanOrEqual(1);

            let onChainBlockNumber = 0;
            let checkpointBlockNumber = 0;
            for (const node of response.data.checkpoints.nodes) {
                // we only have active attestors for Anvil 1
                expect(node.chainKey).toEqual(chain_Anvil1_Key);
                // commit_attestation() contains ensure_none(origin)?;
                // whoId is Origin::none()
                expect(node.whoId).toBeTruthy();

                // these increase for every entity
                expect(node.atBlockNumber).toBeGreaterThan(onChainBlockNumber);
                onChainBlockNumber = node.atBlockNumber;
                expect(node.blockNumber).toBeGreaterThanOrEqual(checkpointBlockNumber);
                checkpointBlockNumber = node.blockNumber;

                expect(node.digest).toBeTruthy();

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { checkpoint(id: "${node.id}") { id, whoId, atBlockNumber, chainKey, blockNumber, digest } }`,
                );
                expect(response2.data.checkpoint).toBeTruthy();
                expect(response2.data.checkpoint.id).toEqual(node.id);
                expect(response2.data.checkpoint.whoId).toEqual(node.whoId);
                expect(response2.data.checkpoint.atBlockNumber).toEqual(node.atBlockNumber);
                expect(response2.data.checkpoint.blockNumber).toEqual(node.blockNumber);
                expect(response2.data.checkpoint.chainKey).toEqual(node.chainKey);
                expect(response2.data.checkpoint.digest).toEqual(node.digest);
            }
        });

        it('graphQL returns updated AttestationChainData', async () => {
            let foundMatch = false;
            const response = await graphQLQuery(
                `query { attestationChainData(orderBy: CHAIN_KEY_ASC, last: 10) { nodes { id, chainKey, lastCheckpointHeaderNumber }}}`,
            );
            for (const node of response.data.attestationChainData.nodes) {
                if (node.chainKey === chain_Anvil1_Key) {
                    foundMatch = true;
                    expect(node.lastCheckpointHeaderNumber).toEqual(lastBlockNumber);
                }
            }
            expect(foundMatch).toBeTruthy();
        });
    });
});
