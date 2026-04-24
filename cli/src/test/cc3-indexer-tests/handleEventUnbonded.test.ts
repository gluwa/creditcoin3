import { newApi, ApiPromise, KeyringPair, BN } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

// Non-zero bond so that Unbonded event is actually emitted during unregisterAttestor.
const TEST_BOND_AMOUNT = new BN('1000000000000000000'); // 1 attest-coin (18 decimals)

describe('handleEventUnbonded()', () => {
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

        // Set a non-zero minBondRequirement so Unbonded event fires on unregister.
        await api.tx.sudo
            .sudo(
                api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, TEST_BOND_AMOUNT.toString()),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await forElapsedBlocks(api, { minBlocks: 2 });

        // register here just so we can unregister a bit later
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 60_000);

    afterAll(async () => {
        // Reset minBondRequirement to 0 to avoid affecting other tests.
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        await api.tx.sudo
            .sudo(api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, '0'))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await api.disconnect();
    });

    describe('when new attestor is unregistered', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);

            // NOTE: unregistering the attestor will also unbond (Unbonded event fires because bond > 0)
            await api.tx.attestation
                .unregisterAttestor(chain_Anvil2_Key, attestor.address)
                .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
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
                if (node.stashId === bob.address && BigInt(node.blockNumber) > startingBlock) {
                    foundMatch = true;
                }
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(BigInt(node.blockNumber)).toBeGreaterThan(0n);

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
