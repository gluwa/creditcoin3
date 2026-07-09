import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import { OutboxContract, OutboxMessage, TransactionVerified } from '../types';
import { createOutboxDatasource } from '../types';

// Event signature for Native Query Verifier precompile
// TransactionVerified(uint64 indexed chainKey, uint64 indexed height, uint64 transactionIndex)
// Note: chainKey and height are indexed (in topics), transactionIndex is in data
type TransactionVerifiedArgs = [bigint, bigint, bigint];

export async function handleTransactionVerified(event: FrontierEvmEvent<TransactionVerifiedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for TransactionVerified event`);
        return;
    }

    // Event structure: TransactionVerified(uint64 indexed chainKey, uint64 indexed height, uint64 transactionIndex)
    // Topics[0] = event signature hash
    // Topics[1] = chainKey (indexed)
    // Topics[2] = height (indexed)
    // Data = transactionIndex (uint64)
    const [chainKey, height, transactionIndex] = event.args;

    logger.info(`Transaction verified: chainKey=${chainKey}, height=${height}, transactionIndex=${transactionIndex}`);

    // Validate that transaction hash is present - every EVM event originates from a transaction
    if (!event.transactionHash) {
        logger.error(
            `Transaction hash is missing for TransactionVerified event at block ${event.blockNumber}, transactionIndex ${event.transactionIndex}. Skipping record.`,
        );
        return;
    }

    // Create a unique ID for this verification event
    const id = `${event.blockNumber}-${event.transactionIndex}-${event.logIndex || 0}`;

    // Store the verification event
    // The TransactionVerified event contains: chainKey, height, and transactionIndex
    const verification = TransactionVerified.create({
        id,
        chainId: BigInt(chainKey),
        height: BigInt(height),
        transactionIndex: BigInt(transactionIndex), // Transaction index from the event
        ccBlockNumber: BigInt(event.blockNumber), // Creditcoin3 block number when verification occurred
        timestamp: event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now()),
        txHash: event.transactionHash, // Transaction hash at which the event occurred
    });

    await verification.save();
}

// USC write-ability: dynamically-discovered Outbox contracts on Creditcoin L1.
// OutboxFactory: OutboxCreated(bytes32 indexed chainKey, address indexed outboxAddress)
type OutboxCreatedArgs = [string, string];
// Outbox: MessagePublished(bytes32 indexed messageId, address indexed emitterAddress, bool requiresAck, bytes payload)
type MessagePublishedArgs = [string, string, boolean, string];
// Outbox: MessageAcknowledged(bytes32 indexed messageId)
type MessageAcknowledgedArgs = [string];

function eventTimestamp(event: { blockTimestamp?: Date }): bigint {
    return event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now());
}

export async function handleOutboxCreated(event: FrontierEvmEvent<OutboxCreatedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for OutboxCreated event at block ${event.blockNumber}`);
        return;
    }
    if (!event.transactionHash) {
        logger.error(`Transaction hash missing for OutboxCreated at block ${event.blockNumber}. Skipping.`);
        return;
    }

    const [chainKey, outboxAddress] = event.args;
    const address = outboxAddress.toLowerCase();
    // event.address is the factory that emitted OutboxCreated.
    const factoryId = event.address ? event.address.toLowerCase() : undefined;

    logger.info(`OutboxCreated: outbox=${address}, chainKey=${chainKey}, factory=${factoryId}`);

    const outbox = OutboxContract.create({
        id: address,
        chainKey,
        factoryId,
        createdAt: BigInt(event.blockNumber),
        createdTimestamp: eventTimestamp(event),
        createdTxHash: event.transactionHash,
    });
    await outbox.save();

    // Spin up a dynamic datasource that indexes this Outbox's messages. `{ address }` is spread
    // into the 'Outbox' template's processor.options, binding it to this instance.
    await createOutboxDatasource({ address });
}

export async function handleMessagePublished(event: FrontierEvmEvent<MessagePublishedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for MessagePublished event at block ${event.blockNumber}`);
        return;
    }
    if (!event.transactionHash) {
        logger.error(`Transaction hash missing for MessagePublished at block ${event.blockNumber}. Skipping.`);
        return;
    }
    // Defensive: EVM logs always carry the emitting contract address, but if it were ever absent,
    // skip loudly rather than save a message with an empty outboxId — that would be an OutboxMessage
    // dangling outside every OutboxContract relation, silently invisible to by-outbox queries.
    if (!event.address) {
        logger.error(`Contract address missing for MessagePublished at block ${event.blockNumber}. Skipping.`);
        return;
    }

    const [messageId, emitterAddress, requiresAck, payload] = event.args;
    logger.info(`MessagePublished: messageId=${messageId}, emitter=${emitterAddress}, requiresAck=${requiresAck}`);

    // Keyed by messageId so handleMessageAcknowledged can load-and-update the same record.
    // outboxId references the OutboxContract created by handleOutboxCreated (same lowercased address).
    const message = OutboxMessage.create({
        id: messageId,
        outboxId: event.address.toLowerCase(),
        emitter: emitterAddress,
        requiresAck,
        payload,
        publishedAt: BigInt(event.blockNumber),
        publishedTimestamp: eventTimestamp(event),
        publishedTxHash: event.transactionHash,
        acknowledged: false,
        acknowledgedAt: undefined,
        acknowledgedTimestamp: undefined,
        acknowledgedTxHash: undefined,
    });

    await message.save();
}

export async function handleMessageAcknowledged(event: FrontierEvmEvent<MessageAcknowledgedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for MessageAcknowledged event at block ${event.blockNumber}`);
        return;
    }

    const [messageId] = event.args;
    logger.info(`MessageAcknowledged: messageId=${messageId}`);

    // The publish is always seen first when the Outbox is indexed from its creation block. If it is
    // missing, the indexer's start block is after the publish — log and skip rather than fabricate a
    // record with no publish metadata (the entity's publish fields are non-null by design).
    const message = await OutboxMessage.get(messageId);
    if (!message) {
        logger.warn(`MessageAcknowledged for unindexed message ${messageId} (publish before start block?) — skipping`);
        return;
    }

    message.acknowledged = true;
    message.acknowledgedAt = BigInt(event.blockNumber);
    message.acknowledgedTimestamp = eventTimestamp(event);
    message.acknowledgedTxHash = event.transactionHash ?? undefined;

    await message.save();
}
