import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { waitEras } from '../integration-tests/helpers';
import { forElapsedBlocks, randomIntBetween } from '../utils';
import { graphQLQuery } from './common';

describe('handleVoteAcceptanceWindowChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    // avoid the default of 3
    const newVoteAcceptanceWindow = BigInt(randomIntBetween(1, 2));
    // unique integer to serve as chain id during testing
    const newChainId = Date.now();
    const newChainName = `Test Chain ${newChainId}`;
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
                    null,
                    null,
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

    describe('when new vote acceptance window is set', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.attestation.setVoteAcceptanceWindow(newChainKey, newVoteAcceptanceWindow))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            // wait for the pending change to take effect
            await waitEras(1, api);

            // wait for indexer to index this event
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 300_000);

        it('graphQL returns known VoteAcceptanceWindowChanged entity', async () => {
            const response = await graphQLQuery(
                `query { 
                    voteAcceptanceWindowChangeds(
                        orderBy: BLOCK_NUMBER_ASC,
                        last: 1,
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                    ) { 
                        nodes { id, blockNumber, date, chainKey, voteAcceptanceWindow }
                    }
                }`,
            );
            expect(response.data.voteAcceptanceWindowChangeds.nodes).toBeTruthy();
            expect(response.data.voteAcceptanceWindowChangeds.nodes.length).toEqual(1);

            for (const node of response.data.voteAcceptanceWindowChangeds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.chainKey)).toEqual(newChainKey);
                expect(BigInt(node.voteAcceptanceWindow)).toEqual(newVoteAcceptanceWindow);
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
                        nodes { id, voteAcceptanceWindow }
                    }
                }`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(BigInt(node.voteAcceptanceWindow)).toEqual(newVoteAcceptanceWindow);
            }
        });
    });
});
