import { newApi, ApiPromise, KeyringPair, BN } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount, mintAttestCoin, setMinBondRequirement } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

/**
 * `Unbonded` carries the bond value and is only emitted for a non-zero bond, but
 * `DefaultMinBondRequirement` is 0. So this suite mints attest coin to a dedicated stash and raises
 * the chain's requirement for the lifetime of the suite (restored in `afterAll`).
 */
const TEST_BOND = new BN('100000000000000000000'); // 100 units

describe('handleEventUnbonded()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    /** Dedicated stash so `stashId` matching is exact and no other suite's ledger interferes. */
    let stash: any;
    let attestor: any;
    let startingBlock: bigint;
    let previousMinBond: string | undefined;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        stash = await randomFundedAccount(api, root);
        attestor = await randomFundedAccount(api, root);

        // Bond collateral must exist before `register_attestor` moves it into the bond pool.
        await mintAttestCoin(api, root, stash.address, TEST_BOND.muln(4));
        previousMinBond = await setMinBondRequirement(api, root, chain_Anvil2_Key, TEST_BOND);

        // register here just so we can unregister a bit later
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(stash.keyring, { nonce: await api.rpc.system.accountNextIndex(stash.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 90_000);

    afterAll(async () => {
        try {
            // `MinBondRequirement` is chain-wide state shared with every other suite on this node.
            if (previousMinBond !== undefined) {
                await setMinBondRequirement(api, root, chain_Anvil2_Key, previousMinBond);
            }
        } finally {
            await api.disconnect();
        }
    });

    describe('when new attestor is unregistered', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);

            // NOTE: unregistering the attestor will also unbond
            await api.tx.attestation
                .unregisterAttestor(chain_Anvil2_Key, attestor.address)
                .signAndSend(stash.keyring, { nonce: await api.rpc.system.accountNextIndex(stash.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known Unbonded entity', async () => {
            const response = await graphQLQuery(
                `query { unbondeds (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
            );
            expect(response.data.unbondeds.nodes).toBeTruthy();
            expect(response.data.unbondeds.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.unbondeds.nodes) {
                expect(BigInt(node.amount)).toBeGreaterThan(0n);
                expect(node.stashId).toBeTruthy();
                expect(node.whoId).toBeTruthy();
                expect(node.whoId).toEqual(node.stashId);
                // This suite owns `stash`, so a match is exact rather than best-effort.
                if (node.stashId === stash.address && BigInt(node.blockNumber) > startingBlock) {
                    expect(BigInt(node.amount)).toEqual(BigInt(TEST_BOND.toString()));
                    foundMatch = true;
                }
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.blockNumber)).toBeGreaterThan(0n);

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { unbonded(id: "${node.id}") { id, amount, stashId, whoId, date, blockNumber } }`,
                );
                expect(response2.data.unbonded).toBeTruthy();
                expect(response2.data.unbonded.id).toEqual(node.id);
                expect(response2.data.unbonded.amount).toEqual(node.amount);
                expect(response2.data.unbonded.stashId).toEqual(node.stashId);
                expect(response2.data.unbonded.whoId).toEqual(node.whoId);
                expect(response2.data.unbonded.date).toEqual(node.date);
                expect(response2.data.unbonded.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
