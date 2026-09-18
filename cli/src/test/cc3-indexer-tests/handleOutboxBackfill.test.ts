import { U64 } from '@polkadot/types-codec';
import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { deployContract } from '../blockchain-tests/helpers';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// Late factory registration cannot retroactively authorize publications. When governance later
// configures a Discovery with existing members, its snapshot admits the Outbox for new messages.
describe('Outbox Discovery admission after deployment', () => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let root: KeyringPair;
    let alith: ethers.Wallet;
    let contract: ethers.Contract;
    let outboxAddress: string;
    let startingBlock: bigint;
    let chainKey: U64;
    let chainKeyNumber: number;
    let chainKeyBytes32: string;

    const chainId = BigInt(Date.now());
    const chainName = `Outbox Backfill Chain ${chainId}`;
    const encoding = 'V1';

    const validator = '0x00000000000000000000000000000000000000ac';
    const version = '1.1';

    const messageId = ethers.zeroPadValue(ethers.toBeHex(BigInt(Date.now()) + 1n), 32);
    const authorizedMessageId = ethers.zeroPadValue(ethers.toBeHex(BigInt(messageId) + 1n), 32);
    const emitterAddress = '0x00000000000000000000000000000000000000e2';
    const emitterBytes32 = ethers.zeroPadBytes(emitterAddress, 32);
    const payload = ethers.hexlify(ethers.toUtf8Bytes('cc3-indexer outbox backfill'));

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        startingBlock = BigInt((await getChainStatus(api)).bestNumber);
        expect(startingBlock).toBeGreaterThan(0n);

        contract = await deployContract('MockWriteAbilityEmitter', [], alith);
        outboxAddress = (await contract.getAddress()).toLowerCase();

        // The chain key must exist — quarantine is gated on it (an unknown key is never
        // quarantined, so there would be nothing to promote and this suite would test the void).
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

        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
        expect(chainKey.toBigInt()).toBeGreaterThan(0n);
        expect(chainKey.toBigInt()).toBeLessThan(4_294_967_296n);
        chainKeyNumber = Number(chainKey.toBigInt());
        chainKeyBytes32 = `0x${chainKey.toBigInt().toString(16).padStart(64, '0')}`;

        // Deliberately NO setOutboxFactoryAddr yet — that is the whole point of this suite.
        // Emit the complete lifecycle while unauthorized.
        let tx = await contract.getFunction('emitOutboxCreated')(chainKeyNumber, validator, version, {
            gasLimit: 1_000_000,
        });
        await tx.wait();
        tx = await contract.getFunction('emitMessagePublished')(messageId, emitterBytes32, true, payload, {
            gasLimit: 1_000_000,
        });
        await tx.wait();
        tx = await contract.getFunction('emitMessageAcknowledged')(messageId, {
            gasLimit: 1_000_000,
        });
        await tx.wait();

        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 240_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(chainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
        await provider.destroy();
    });

    describe('while the factory is unregistered', () => {
        it('does not admit an OutboxContract', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(response.data.outboxContracts.nodes).toEqual([]);
        });

        it('quarantines the announcement as a PendingOutbox', async () => {
            const response = await graphQLQuery(
                `query {
                    pendingOutboxes(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id, chainKey, factoryAddress, chainKeyBytes32, createdAt, createdTxHash }}}`,
            );
            expect(response.data.pendingOutboxes.nodes.length).toEqual(1);
            const node = response.data.pendingOutboxes.nodes[0];
            expect(BigInt(node.chainKey)).toEqual(chainKey.toBigInt());
            // The mock announces itself, so it is its own emitter/factory.
            expect(node.factoryAddress).toEqual(outboxAddress);
            expect(node.chainKeyBytes32).toEqual(chainKeyBytes32);
            expect(BigInt(node.createdAt)).toBeGreaterThanOrEqual(startingBlock);
        });

        it('does not retain unauthorized messages for later promotion', async () => {
            const response = await graphQLQuery(
                `query {
                    quarantinedMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes {
                        id, outboxAddress, emitter, canAck, payload,
                        publishedAt, acknowledged, acknowledgedAt
                    }}}`,
            );
            expect(response.data.quarantinedMessages.nodes).toEqual([]);
        });

        it('does not index the message', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(response.data.outboxMessages.nodes).toEqual([]);
        });
    });

    describe('when governance registers the factory afterwards', () => {
        beforeAll(async () => {
            await api.tx.sudo
                .sudo(api.tx.supportedChains.setOutboxFactoryAddr(chainKey, outboxAddress))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('still does not admit the permissionlessly deployed Outbox', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id, chainKey, factoryId, createdAt, createdTimestamp, createdTxHash }}}`,
            );
            expect(response.data.outboxContracts.nodes).toEqual([]);
        });

        it('does not backfill the unauthorized message', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes {
                        id, outboxId, emitter, canAck, payload,
                        publishedAt, publishedTxHash,
                        acknowledged, acknowledgedAt, acknowledgedTxHash
                    }}}`,
            );
            expect(response.data.outboxMessages.nodes).toEqual([]);
        });

        it('keeps only the bounded candidate announcement', async () => {
            const pending = await graphQLQuery(
                `query {
                    pendingOutboxes(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(pending.data.pendingOutboxes.nodes).toEqual([{ id: outboxAddress }]);

            const quarantined = await graphQLQuery(
                `query {
                    quarantinedMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(quarantined.data.quarantinedMessages.nodes).toEqual([]);
        });
    });

    describe('when governance later registers a Discovery containing the Outbox', () => {
        beforeAll(async () => {
            const discovery = await deployContract('MockOutboxDiscovery', [], alith);
            const register = await discovery.getFunction('registerOutbox')(chainKeyNumber, outboxAddress, {
                gasLimit: 1_000_000,
            });
            await register.wait();
            await forElapsedBlocks(api, { minBlocks: 3 });
            await api.tx.sudo
                .sudo(api.tx.supportedChains.setOutboxDiscoveryAddr(chainKey, await discovery.getAddress()))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
            const publish = await contract.getFunction('emitMessagePublished')(
                authorizedMessageId,
                emitterBytes32,
                false,
                payload,
                { gasLimit: 1_000_000 },
            );
            await publish.wait();
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 180_000);

        it('snapshots the member and indexes only its newly authorized publication', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(filter: { id: { equalTo: "${outboxAddress}" }}) { nodes { id, chainKey } }
                    outboxMessages(filter: { outboxId: { equalTo: "${outboxAddress}" }}) { nodes { id } }
                    pendingOutboxes(filter: { id: { equalTo: "${outboxAddress}" }}) { nodes { id } }
                }`,
            );
            expect(response.data.outboxContracts.nodes).toEqual([{ id: outboxAddress, chainKey: chainKeyBytes32 }]);
            expect(response.data.outboxMessages.nodes).toEqual([{ id: authorizedMessageId }]);
            expect(response.data.pendingOutboxes.nodes).toEqual([]);
        });
    });
});
