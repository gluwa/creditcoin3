import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { deployContract } from '../blockchain-tests/helpers';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// USC write-ability EVM handlers, exercised end to end in publish order:
//   handleOutboxCreated  -> OutboxContract (+ a dynamic datasource for that address)
//   handleMessagePublished -> OutboxMessage
//   handleMessageAcknowledged -> flips the same OutboxMessage to acknowledged
//
// The events come from MockWriteAbilityEmitter rather than the real fee stack: the indexer
// discovers Outboxes by *topic* chain-wide (see `outboxDiscoveryDatasource` in
// cc3-indexer/datasources.ts), so an unregistered emitter is precisely the unauthenticated
// discovery path this covers. The mock announces itself as the Outbox, so the datasource created
// from OutboxCreated is the one that then picks up its MessagePublished / MessageAcknowledged.
//
// The three steps must land in this order and be indexed between each: the dynamic datasource only
// exists once OutboxCreated has been processed, and the ack only updates an already-indexed message.
describe('Outbox lifecycle handlers', () => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let contract: ethers.Contract;
    let outboxAddress: string;
    let startingBlock: bigint;

    // uint32 chain key, unique per run so this test can never interact with another record's
    // per-chain-key unauthenticated-datasource cap (audit P2-1).
    const chainKey = Number(BigInt(Date.now()) % 4_000_000_000n);
    // The bytes32 form the handler stores: `bytes32(uint256(chainKey))`.
    const chainKeyBytes32 = `0x${chainKey.toString(16).padStart(64, '0')}`;

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

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        const alith = new ethers.Wallet(privateKey).connect(provider);

        startingBlock = BigInt((await getChainStatus(api)).bestNumber);
        expect(startingBlock).toBeGreaterThan(0n);

        contract = await deployContract('MockWriteAbilityEmitter', [], alith);
        // The handler lowercases the announced address to key OutboxContract.
        outboxAddress = (await contract.getAddress()).toLowerCase();
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    describe('when an outbox announces itself', () => {
        beforeAll(async () => {
            const tx = await contract.getFunction('emitOutboxCreated')(chainKey, validator, version, {
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
                // `factoryId` records whichever contract *emitted* OutboxCreated, unconditionally —
                // it is not evidence that the emitter is a registered OutboxFactory (that is the
                // separate `authenticated` trust signal, which only relaxes the DoS cap and is not
                // persisted). The mock announces itself as the Outbox, so it is its own emitter and
                // factoryId equals the id here.
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
                        id, outboxId, emitter, requiresAck, payload,
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
                expect(node.requiresAck).toEqual(true);
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
});
