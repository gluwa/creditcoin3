import { newApi, ApiPromise, KeyringPair, BN } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount, waitEras } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

// Non-zero bond so that Unbonded/Withdrawn events are actually emitted.
const TEST_BOND_AMOUNT = new BN('1000000000000000000'); // 1 attest-coin (18 decimals)

describe('handleEventWithdrawn()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let attestor: any;
    let startingBlock: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // Give bob attest-coin so register succeeds with a non-zero bond requirement.
        await api.tx.sudo
            .sudo(api.tx.attestation.forceMintBondAsset(bob.address, TEST_BOND_AMOUNT.toString()))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        // Set a non-zero minBondRequirement so Unbonded/Withdrawn events fire.
        await api.tx.sudo
            .sudo(
                api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, TEST_BOND_AMOUNT.toString()),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await forElapsedBlocks(api, { minBlocks: 2 });

        // register & bond
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // unregister & unbond
        await api.tx.attestation
            .unregisterAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        // wait for funds to be unlocked!
        const unbondingPeriod: number = api.consts.attestation.bondingDuration.toNumber();
        await waitEras(unbondingPeriod, api); // ~ 5 minutes
    }, 450_000);

    afterAll(async () => {
        // Reset minBondRequirement to 0 to avoid affecting other tests.
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        await api.tx.sudo
            .sudo(api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, '0'))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await api.disconnect();
    });

    describe('when funds are withdrawn', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);

            await api.tx.attestation
                .withdrawUnbonded()
                .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
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
                if (node.stashId === bob.address && BigInt(node.blockNumber) > startingBlock) {
                    foundMatch = true;
                }
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.blockNumber)).toBeGreaterThan(0n);

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
