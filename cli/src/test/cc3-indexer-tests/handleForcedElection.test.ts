import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

describe('handleForcedElection()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;

    const epoch = 999;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when a force election is triggered', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.attestation.forceElection(epoch))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 2 });
        }, 30_000);

        it('graphQL returns ForcedElection entity', async () => {
            const response = await graphQLQuery(
                `query { forcedElections(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, epoch }}}`,
            );
            expect(response.data.forcedElections.nodes).toBeTruthy();
            expect(response.data.forcedElections.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.forcedElections.nodes) {
                expect(node.id).toBeTruthy();
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(Number(node.epoch)).toEqual(epoch);
            }
        });
    });
});
