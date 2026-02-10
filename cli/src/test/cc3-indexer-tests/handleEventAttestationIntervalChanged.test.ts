import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { waitEras } from '../integration-tests/helpers';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';

describe('handleEventAttestationIntervalChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    // avoid the default of 10
    const newInterval = BigInt(randomIntBetween(11, 21));
    // unique integer to serve as chain id during testing
    const newChainId = Date.now();
    const newChainName = `Test Chain ${newChainId}`;
    const encoding = 'V1';
    let newChainKey = 0n;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(
                    newChainId,
                    newChainName,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    encoding,
                ),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // will fail if the query returns None
        newChainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(newChainId, newChainName))
            .unwrap()
            .toBigInt();
        expect(newChainKey).toBeGreaterThan(0n);
    }, 45_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(newChainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
    });

    describe('when new chain attestation interval is set', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            // NOTE: by defauilt it is 10
            await api.tx.sudo
                .sudo(api.tx.attestation.setChainAttestationInterval(newChainKey, newInterval))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
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
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(BigInt(node.interval)).toEqual(newInterval);
            }
        });

        it('graphQL returns updated AttestationChainData entity', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationChainData(
                        orderBy: CHAIN_KEY_ASC,
                        last: 1,
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                    ) {
                        nodes { id, attestationInterval }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(BigInt(node.attestationInterval)).toEqual(newInterval);
            }
        });
    });
});
