import { newApi, ApiPromise, KeyringPair, BN } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount, waitEras, mintAttestCoin, setMinBondRequirement } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

/**
 * `Withdrawn` carries the withdrawn bond and is only emitted for a non-zero bond, but
 * `DefaultMinBondRequirement` is 0. So this suite mints attest coin to a dedicated stash and raises
 * the chain's requirement for the lifetime of the suite (restored in `afterAll`).
 */
const TEST_BOND = new BN('100000000000000000000'); // 100 units

describe('handleEventWithdrawn()', () => {
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

        // register & bond
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(stash.keyring, { nonce: await api.rpc.system.accountNextIndex(stash.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // unregister & unbond
        await api.tx.attestation
            .unregisterAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(stash.keyring, { nonce: await api.rpc.system.accountNextIndex(stash.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // wait for funds to be unlocked!
        const unbondingPeriod: number = api.consts.attestation.bondingDuration.toNumber();
        await waitEras(unbondingPeriod, api); // ~ 5 minutes
    }, 450_000);

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

    describe('when funds are withdrawn', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);

            await api.tx.attestation
                .withdrawUnbonded()
                .signAndSend(stash.keyring, { nonce: await api.rpc.system.accountNextIndex(stash.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known Withdrawn entity', async () => {
            const response = await graphQLQuery(
                `query { withdrawns (orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, amount, stashId, whoId, date, blockNumber }}}`,
            );
            expect(response.data.withdrawns.nodes).toBeTruthy();
            expect(response.data.withdrawns.nodes.length).toBeGreaterThanOrEqual(1);

            let foundMatch = false;
            for (const node of response.data.withdrawns.nodes) {
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
                    `query { withdrawn(id: "${node.id}") { id, amount, stashId, whoId, date, blockNumber } }`,
                );
                expect(response2.data.withdrawn).toBeTruthy();
                expect(response2.data.withdrawn.id).toEqual(node.id);
                expect(response2.data.withdrawn.amount).toEqual(node.amount);
                expect(response2.data.withdrawn.stashId).toEqual(node.stashId);
                expect(response2.data.withdrawn.whoId).toEqual(node.whoId);
                expect(response2.data.withdrawn.date).toEqual(node.date);
                expect(response2.data.withdrawn.blockNumber).toEqual(node.blockNumber);
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
