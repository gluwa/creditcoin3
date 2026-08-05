import { U64 } from '@polkadot/types-codec';
import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { deployContract } from '../blockchain-tests/helpers';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// USC write-ability EVM handlers, exercised end to end in publish order:
//   handleOutboxCreated  -> OutboxContract (the admission the message handlers key on)
//   handleMessagePublished -> OutboxMessage
//   handleMessageAcknowledged -> flips the same OutboxMessage to acknowledged
//
// The events come from MockWriteAbilityEmitter, which announces itself as the Outbox; all three
// handlers are chain-wide topic watches that authorize each event against store state (no dynamic
// datasources).
//
// Discovery is fail-closed: `handleOutboxCreated` only *admits* an `OutboxCreated` whose emitter is
// the factory governance registered for that raw chain key. This suite covers the ordered path
// (register the factory first, as the deploy tooling does); the reverse order — announce first,
// register later — is the backfill path covered by handleOutboxBackfill.test.ts.
//
// The three steps must land in this order and be indexed between each: admission only exists once
// OutboxCreated has been processed, and the ack only updates an already-indexed message.
describe('Outbox lifecycle handlers', () => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let root: KeyringPair;
    let alith: ethers.Wallet;
    let contract: ethers.Contract;
    let outboxAddress: string;
    let startingBlock: bigint;
    // Assigned in beforeAll from the pallet's monotonic uniq-key counter, so it is a small integer
    // that fits the `uint32 chainKey` the OutboxCreated event carries.
    let chainKey: U64;
    let chainKeyNumber: number;
    // The bytes32 form the handler stores: `bytes32(uint256(chainKey))`.
    let chainKeyBytes32: string;

    const chainId = BigInt(Date.now());
    const chainName = `Outbox Lifecycle Chain ${chainId}`;
    const encoding = 'V1';

    const validator = '0x00000000000000000000000000000000000000ac';
    const version = '1.1';

    const messageId = ethers.zeroPadValue(ethers.toBeHex(BigInt(Date.now())), 32);
    const emitterAddress = '0x00000000000000000000000000000000000000e1';
    // `emitterAddress` travels as bytes32(bytes20(emitter)) — the address in the HIGH bytes, i.e.
    // right-padded with 12 zero bytes. `zeroPadBytes` pads on the right (`zeroPadValue` pads left).
    const emitterBytes32 = ethers.zeroPadBytes(emitterAddress, 32);
    const payload = ethers.hexlify(ethers.toUtf8Bytes('cc3-indexer outbox lifecycle'));

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        startingBlock = BigInt((await getChainStatus(api)).bestNumber);
        expect(startingBlock).toBeGreaterThan(0n);

        contract = await deployContract('MockWriteAbilityEmitter', [], alith);
        // The handler lowercases the announced address to key OutboxContract.
        outboxAddress = (await contract.getAddress()).toLowerCase();

        // A real chain, so the uniq key exists on-chain and `setOutboxFactoryAddr` is accepted.
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
        // The event field is a uint32; the pallet allocates keys from a monotonic counter, so this
        // holds in practice. Assert it rather than truncate silently if that ever stops being true.
        expect(chainKey.toBigInt()).toBeLessThan(4_294_967_296n);
        chainKeyNumber = Number(chainKey.toBigInt());
        chainKeyBytes32 = `0x${chainKey.toBigInt().toString(16).padStart(64, '0')}`;

        // Register the mock as this chain key's Outbox factory, so its OutboxCreated is accepted.
        await api.tx.sudo
            .sudo(api.tx.supportedChains.setOutboxFactoryAddr(chainKey, outboxAddress))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });

        const stored = await api.query.supportedChains.outboxFactories(chainKey);
        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().toString().toLowerCase()).toEqual(outboxAddress);

        // The registration must be *indexed* before OutboxCreated is emitted — the handler reads
        // OutboxFactoryRegistration, so emitting first would be rejected as unauthenticated.
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 180_000);

    afterAll(async () => {
        await api.tx.sudo
            .sudo(api.tx.supportedChains.removeChain(chainKey, true))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });

        await api.disconnect();
        await provider.destroy();
    });

    describe('when an outbox announces itself', () => {
        beforeAll(async () => {
            const tx = await contract.getFunction('emitOutboxCreated')(chainKeyNumber, validator, version, {
                gasLimit: 1_000_000,
            });
            await tx.wait();

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known OutboxContract entity', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id, chainKey, factoryId, createdAt, createdTimestamp, createdTxHash }}}`,
            );
            expect(response.data.outboxContracts.nodes).toBeTruthy();
            expect(response.data.outboxContracts.nodes.length).toEqual(1);

            for (const node of response.data.outboxContracts.nodes) {
                expect(node.id).toEqual(outboxAddress);
                expect(node.chainKey).toEqual(chainKeyBytes32);
                expect(BigInt(node.createdAt)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.createdTimestamp)).toBeGreaterThan(0n);
                expect(node.createdTxHash.startsWith('0x')).toEqual(true);
                // `factoryId` records the contract that emitted OutboxCreated, which discovery now
                // requires to be the registered factory. The mock announces itself as the Outbox, so
                // it is its own emitter and factoryId equals the id here.
                expect(node.factoryId).toEqual(outboxAddress);
            }
        });
    });

    describe('when a message is published on that outbox', () => {
        beforeAll(async () => {
            const tx = await contract.getFunction('emitMessagePublished')(messageId, emitterBytes32, true, payload, {
                gasLimit: 1_000_000,
            });
            await tx.wait();

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known OutboxMessage entity', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes {
                        id, outboxId, emitter, canAck, payload,
                        publishedAt, publishedTimestamp, publishedTxHash, acknowledged
                    }}}`,
            );
            expect(response.data.outboxMessages.nodes).toBeTruthy();
            expect(response.data.outboxMessages.nodes.length).toEqual(1);

            for (const node of response.data.outboxMessages.nodes) {
                expect(node.id).toEqual(messageId);
                // Relation back to the OutboxContract created above.
                expect(node.outboxId).toEqual(outboxAddress);
                // The bytes32 emitter is unwrapped back to a plain 20-byte address.
                expect(node.emitter).toEqual(emitterAddress);
                expect(node.canAck).toEqual(true);
                expect(node.payload).toEqual(payload);
                expect(BigInt(node.publishedAt)).toBeGreaterThanOrEqual(startingBlock);
                expect(BigInt(node.publishedTimestamp)).toBeGreaterThan(0n);
                expect(node.publishedTxHash.startsWith('0x')).toEqual(true);
                // Not acknowledged yet — that is the next step.
                expect(node.acknowledged).toEqual(false);
            }
        });
    });

    describe('when that message is acknowledged', () => {
        beforeAll(async () => {
            const tx = await contract.getFunction('emitMessageAcknowledged')(messageId, {
                gasLimit: 1_000_000,
            });
            await tx.wait();

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns the same OutboxMessage, now acknowledged', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxMessages(
                        filter: { id: { equalTo: "${messageId}" }},
                        last: 1,
                    ) { nodes {
                        id, acknowledged, acknowledgedAt, acknowledgedTimestamp, acknowledgedTxHash,
                        publishedAt
                    }}}`,
            );
            expect(response.data.outboxMessages.nodes).toBeTruthy();
            expect(response.data.outboxMessages.nodes.length).toEqual(1);

            for (const node of response.data.outboxMessages.nodes) {
                // Same record updated in place, not a second row.
                expect(node.id).toEqual(messageId);
                expect(node.acknowledged).toEqual(true);
                expect(BigInt(node.acknowledgedAt)).toBeGreaterThanOrEqual(BigInt(node.publishedAt));
                expect(BigInt(node.acknowledgedTimestamp)).toBeGreaterThan(0n);
                expect(node.acknowledgedTxHash.startsWith('0x')).toEqual(true);
            }
        });
    });

    // The negative half of the same rule, kept last so the ordered three-step lifecycle above stays
    // contiguous. An emitter governance never registered must not be able to create an admitted
    // OutboxContract just by emitting a well-formed OutboxCreated. That is the DoS vector the
    // fail-closed check exists for, so assert it directly rather than leave it implied by the happy
    // path. (It IS quarantined — the chain key exists and a later rotation could authorize it — but
    // quarantine is bounded state, not admission.)
    describe('when an unregistered contract announces itself for the same chain key', () => {
        let impostorAddress: string;

        beforeAll(async () => {
            const impostor = await deployContract('MockWriteAbilityEmitter', [], alith);
            impostorAddress = (await impostor.getAddress()).toLowerCase();
            expect(impostorAddress).not.toEqual(outboxAddress);

            const tx = await impostor.getFunction('emitOutboxCreated')(chainKeyNumber, validator, version, {
                gasLimit: 1_000_000,
            });
            await tx.wait();

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 120_000);

        it('graphQL returns no OutboxContract for the unregistered emitter', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${impostorAddress}" }},
                        last: 1,
                    ) { nodes { id }}}`,
            );
            expect(response.data.outboxContracts.nodes).toEqual([]);
        });

        it('quarantines the impostor as a PendingOutbox instead', async () => {
            const response = await graphQLQuery(
                `query {
                    pendingOutboxes(
                        filter: { id: { equalTo: "${impostorAddress}" }},
                        last: 1,
                    ) { nodes { id, chainKey, factoryAddress }}}`,
            );
            expect(response.data.pendingOutboxes.nodes.length).toEqual(1);
            expect(response.data.pendingOutboxes.nodes[0].factoryAddress).toEqual(impostorAddress);
            expect(BigInt(response.data.pendingOutboxes.nodes[0].chainKey)).toEqual(chainKey.toBigInt());
        });

        it('leaves the registered Outbox untouched', async () => {
            const response = await graphQLQuery(
                `query {
                    outboxContracts(
                        filter: { id: { equalTo: "${outboxAddress}" }},
                        last: 1,
                    ) { nodes { id, factoryId }}}`,
            );
            expect(response.data.outboxContracts.nodes.length).toEqual(1);
            expect(response.data.outboxContracts.nodes[0].factoryId).toEqual(outboxAddress);
        });
    });
});
