import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks, randomIntBetween, sleep } from '../utils';
import { graphQLQuery } from './common';

// Poll the chain until `set_chain_attestation_interval` has actually been *applied* to
// `chainKey`, i.e. `ChainAttestationInterval` reads back as `expected`.
//
// The extrinsic only writes `PendingAttestationInterval`; the value is moved into
// `ChainAttestationInterval` (and `AttestationIntervalChanged` emitted) by
// `apply_interval_updates()`, which runs from `on_new_epoch_randomness` — an **epoch**
// boundary. Waiting a fixed `waitEras(1)` raced that: `signAndSend` is not awaited to
// inclusion, so the boundary could pass *before* the extrinsic landed, leaving the value
// pending with nothing left to wait for. The assertions then read the chain-registration
// record (the only AttestationIntervalChanged ever emitted for that key) and saw the default
// interval. Polling the applied value removes the ordering assumption entirely — it simply
// waits for the next boundary after the write, however the two interleave.
//
// Throws on timeout so a genuinely stuck update still fails the test rather than hanging.
async function waitForIntervalApplied(
    api: ApiPromise,
    chainKey: bigint,
    expected: bigint,
    timeoutMs = 240_000,
    intervalMs = 3_000,
): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    let last = '<none>';

    while (Date.now() < deadline) {
        const applied = (await api.query.attestation.chainAttestationInterval(chainKey)).toBigInt();
        last = applied.toString();
        if (applied === expected) {
            return;
        }
        await sleep(intervalMs);
    }

    throw new Error(
        `attestation interval for chain_key ${chainKey} was not applied within ${timeoutMs}ms ` +
            `(expected ${expected}, last saw ${last}) — the pending update never reached an epoch boundary`,
    );
}

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

    describe('when new chain attestation interval is set', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            // NOTE: by defauilt it is 10
            await api.tx.sudo
                .sudo(api.tx.attestation.setChainAttestationInterval(newChainKey, newInterval))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            // Wait for the pending change to actually be applied on-chain, rather than for a fixed
            // number of eras — see waitForIntervalApplied for why the fixed wait raced.
            await waitForIntervalApplied(api, newChainKey, newInterval);

            // wait for indexer to index this event
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 300_000);

        it('graphQL returns known AttestationIntervalChanged entity', async () => {
            const response = await graphQLQuery(
                `query {
                    attestationIntervalChangeds(
                        filter: { chainKey: { equalTo: "${newChainKey}" }},
                        orderBy: BLOCK_NUMBER_ASC,
                        last: 1
                    ) { nodes { id, blockNumber, date, chainKey, interval }}}`,
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
