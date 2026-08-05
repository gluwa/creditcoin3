import { U64 } from '@polkadot/types-codec';
import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { deployContract } from '../blockchain-tests/helpers';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// The backfill half of fail-closed Outbox discovery (handleOutboxLifecycle.test.ts covers the
// ordered half). Here the deploy ordering is deliberately WRONG: the mock announces its
// OutboxCreated, publishes a message and acknowledges it, all BEFORE governance registers it as the
// chain key's factory. Fail-closed discovery must reject the announcement — but into quarantine
// (PendingOutbox / QuarantinedMessage), not the void. When setOutboxFactoryAddr is finally indexed,
// handleOutboxFactoryRegistered promotes the quarantined Outbox and its full message lifecycle so a
// mis-ordered (or racing — observed live on usc-dev) deployment degrades to "indexed late" instead
// of "never indexed".
describe('Outbox discovery backfill', () => {
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

        it('quarantines the message — with the ack that followed it', async () => {
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
            expect(response.data.quarantinedMessages.nodes.length).toEqual(1);
            const node = response.data.quarantinedMessages.nodes[0];
            expect(node.outboxAddress).toEqual(outboxAddress);
            expect(node.emitter).toEqual(emitterAddress);
            expect(node.canAck).toEqual(true);
            expect(node.payload).toEqual(payload);
            // The ack landed while quarantined and must already be recorded here, or promotion
            // would resurrect the message as unacknowledged.
            expect(node.acknowledged).toEqual(true);
            expect(BigInt(node.acknowledgedAt)).toBeGreaterThanOrEqual(BigInt(node.publishedAt));
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

        it('promotes the quarantined Outbox to an admitted OutboxContract', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id, chainKey, factoryId, createdAt, createdTimestamp, createdTxHash }}}`,
            );
            expect(response.data.outboxContracts.nodes.length).toEqual(1);
            const node = response.data.outboxContracts.nodes[0];
            expect(node.chainKey).toEqual(chainKeyBytes32);
            expect(node.factoryId).toEqual(outboxAddress);
            // Promotion preserves the ORIGINAL creation provenance, not the registration block.
            expect(BigInt(node.createdAt)).toBeGreaterThanOrEqual(startingBlock);
            expect(node.createdTxHash.startsWith('0x')).toEqual(true);
        });

        it('backfills the message with its full lifecycle', async () => {
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
            expect(response.data.outboxMessages.nodes.length).toEqual(1);
            const node = response.data.outboxMessages.nodes[0];
            expect(node.outboxId).toEqual(outboxAddress);
            expect(node.emitter).toEqual(emitterAddress);
            expect(node.canAck).toEqual(true);
            expect(node.payload).toEqual(payload);
            expect(BigInt(node.publishedAt)).toBeGreaterThanOrEqual(startingBlock);
            expect(node.publishedTxHash.startsWith('0x')).toEqual(true);
            expect(node.acknowledged).toEqual(true);
            expect(BigInt(node.acknowledgedAt)).toBeGreaterThanOrEqual(BigInt(node.publishedAt));
            expect(node.acknowledgedTxHash.startsWith('0x')).toEqual(true);
        });

        it('empties the quarantine', async () => {
            const pending = await graphQLQuery(
                `query {
                    pendingOutboxes(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(pending.data.pendingOutboxes.nodes).toEqual([]);

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
});
