import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { waitEras } from '../integration-tests/helpers';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventAttestationIntervalChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: number;
    const newInterval = randomIntBetween(7, 21);

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new chain attestation interval is set', () => {
        beforeAll(async () => {
            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            // NOTE: by defauilt it is 10
            await api.tx.sudo
                .sudo(api.tx.attestation.setChainAttestationInterval(chain_Anvil2_Key, newInterval))
                .signAndSend(root);
            // wait for the pending change to take effect
            await waitEras(1, api);

            // wait for indexer to index this event
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 300_000);

        it('graphQL returns known AttestationIntervalChanged entity', async () => {
            const response = await graphQLQuery(
                `query { attestationIntervalChangeds(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, blockNumber, date, chainKey, interval }}}`,
            );
            expect(response.data.attestationIntervalChangeds.nodes).toBeTruthy();
            expect(response.data.attestationIntervalChangeds.nodes.length).toEqual(1);

            for (const node of response.data.attestationIntervalChangeds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(node.blockNumber).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.chainKey).toEqual(chain_Anvil2_Key);
                expect(node.interval).toEqual(newInterval);
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
                        nodes { id, attestationInterval }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(node.attestationInterval).toEqual(newInterval);
            }
        });
    });
});
