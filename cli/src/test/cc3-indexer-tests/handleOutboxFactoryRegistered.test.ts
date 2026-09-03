import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// USC write-ability: `supportedChains.set_outbox_factory_addr` emits OutboxFactoryRegistered, which
// the indexer records both as a display-oriented OutboxFactory and as the authoritative per-chain
// OutboxFactoryRegistration used to authenticate chain-wide OutboxCreated discovery.
describe('handleOutboxFactoryRegistered()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;
    let chainKey: U64;

    const chainId = BigInt(Date.now());
    const chainName = `Outbox Factory Chain ${chainId}`;
    const encoding = 'V1';

    // A unique lowercase 20-byte EVM address per run: the handler keys OutboxFactory by the
    // lowercased address, so a fixed literal could collide with another suite's factory record.
    const outboxFactoryAddr = `0x${chainId.toString(16).padStart(40, '0')}`;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        startingBlock = BigInt((await getChainStatus(api)).bestNumber);
        expect(startingBlock).toBeGreaterThan(0n);

        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(
                    chainId,
                    chainName,
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

        // Fails loudly if registration did not land, rather than registering the factory against a
        // chain key that does not exist (which the extrinsic would reject as ChainNotSupported).
        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
        expect(chainKey.toBigInt()).toBeGreaterThan(0n);
    }, 60_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(chainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
    });

    describe('when an outbox factory is registered', () => {
        beforeAll(async () => {
            await api.tx.sudo
                .sudo(api.tx.supportedChains.setOutboxFactoryAddr(chainKey, outboxFactoryAddr))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 1 });

            // Confirm on-chain state before asserting on the indexed projection of it.
            const stored = await api.query.supportedChains.outboxFactories(chainKey);
            expect(stored.isSome).toEqual(true);
            expect(stored.unwrap().toString().toLowerCase()).toEqual(outboxFactoryAddr);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known OutboxFactory entity', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxFactories(
                        filter: { id: { equalTo: "${outboxFactoryAddr}" }},
                        last: 1,
                    ) { nodes { id, chainKey, registeredAt, registeredTimestamp }}}`,
            );
            expect(response.data.outboxFactories.nodes).toBeTruthy();
            expect(response.data.outboxFactories.nodes.length).toEqual(1);

            for (const node of response.data.outboxFactories.nodes) {
                // The entity is keyed by the lowercased factory address.
                expect(node.id).toEqual(outboxFactoryAddr);
                expect(BigInt(node.chainKey)).toEqual(chainKey.toBigInt());
                expect(BigInt(node.registeredAt)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.registeredTimestamp)).toBeGreaterThan(0n);
                expect(BigInt(node.registeredTimestamp)).toBeLessThanOrEqual(BigInt(Date.now()));
            }
        });

        it('graphQL returns the exact per-chain factory authorization', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxFactoryRegistrations(
                        filter: { id: { equalTo: "${chainKey.toString()}" }},
                        last: 1,
                    ) { nodes { id, factoryAddress, registeredAt }}}`,
            );
            expect(response.data.outboxFactoryRegistrations.nodes).toEqual([
                expect.objectContaining({
                    id: chainKey.toString(),
                    factoryAddress: outboxFactoryAddr,
                }),
            ]);
            expect(BigInt(response.data.outboxFactoryRegistrations.nodes[0].registeredAt)).toBeGreaterThanOrEqual(
                startingBlock,
            );
        });
    });
});
