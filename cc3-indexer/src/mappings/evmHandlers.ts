import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import { OutboxContract, OutboxDatasourceCount, OutboxFactory, OutboxMessage, TransactionVerified } from '../types';
import { createOutboxDatasource } from '../types';

// Upper bound on how many UNAUTHENTICATED Outbox datasources we will spin up for a single
// write-ability chain key. A legitimate deployment has one Outbox per chain key (a handful across
// redeploys); a large count means a counterfeit contract is emitting `OutboxCreated` to make us
// register unbounded dynamic datasources (audit P2-1 — datasource-creation DoS). Datasources from a
// registered, chain-key-matching factory are trusted and exempt. The cap bounds the blast radius
// without breaking the intentional discover-before-registration flow (see datasources.ts).
const MAX_OUTBOXES_PER_CHAIN_KEY = 32;

// Encode a u64 write-ability chain key as its bytes32 form: `bytes32(uint256(chainKey))`, i.e. the
// 8 big-endian bytes right-aligned in a 32-byte word (matches `chain_key_to_bytes32` in the shared
// `common/write-ability` crate). Used to reconcile an `OutboxFactory.chainKey` (stored as the u64)
// with the bytes32 chain key carried by `OutboxCreated`.
function u64ChainKeyToBytes32(value: bigint): string {
    return '0x' + value.toString(16).padStart(64, '0');
}

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
// OutboxCreated(address indexed outbox, uint32 indexed chainKey, address indexed owner, address validator, string version)
type OutboxCreatedArgs = [string, bigint, string, string, string];
// Outbox: MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload)
// emitterAddress is a bytes32 (20-byte EVM address left-aligned in the high bytes).
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

    // Synced factory event: (outbox, chainKey:uint32, owner, validator, version). chainKey arrives
    // as a number; normalize to the same bytes32 form the factory-correspondence check below uses.
    const [outboxAddress, chainKeyRaw] = event.args;
    const chainKey = u64ChainKeyToBytes32(BigInt(chainKeyRaw));
    const address = outboxAddress.toLowerCase();
    // event.address is the factory that emitted OutboxCreated.
    const factoryId = event.address ? event.address.toLowerCase() : undefined;

    logger.info(`OutboxCreated: outbox=${address}, chainKey=${chainKey}, factory=${factoryId}`);

    // Idempotency guard: reprocessing the same OutboxCreated log (reorg replay, reindex overlap)
    // must not register a SECOND dynamic datasource for the same address — duplicate datasources
    // make every subsequent MessagePublished on this Outbox fire its handler once per duplicate.
    const existing = await OutboxContract.get(address);
    if (existing) {
        logger.warn(`OutboxCreated for already-registered outbox ${address} — skipping duplicate datasource`);
        return;
    }

    // P2-10 — factory correspondence. Discovery watches `OutboxCreated` chain-wide by topic (a
    // factory creates Outboxes *before* it is registered via `OutboxFactoryRegistered`), so we can
    // never hard-require a registered emitter — a counterfeit emitter is simply never a registered
    // OutboxFactory. `authenticated` is therefore a best-effort trust signal, not a gate: it is true
    // only when the emitter IS a registered factory AND (one of) its registered chain key(s) matches
    // the event's bytes32 chain key. NOTE: one factory address may legitimately serve multiple chain
    // keys (the pallet maps chain_key -> factory), and `OutboxFactory` records only the first, so a
    // mismatch is logged but is NOT treated as fatal — dropping the Outbox here would strand a
    // legitimate multi-key factory's messages (a liveness bug worse than the ~zero authentication
    // this check buys). The trust signal only relaxes the DoS cap below.
    let authenticated = false;
    if (factoryId) {
        const factory = await OutboxFactory.get(factoryId);
        if (factory) {
            if (u64ChainKeyToBytes32(factory.chainKey) === chainKey) {
                authenticated = true;
            } else {
                logger.warn(
                    `OutboxCreated from registered factory ${factoryId} for chainKey ${chainKey} does not ` +
                        `match its recorded chainKey ${factory.chainKey.toString()} — indexing anyway ` +
                        `(a factory may serve multiple chain keys), but not exempting it from the DoS cap`,
                );
            }
        }
    }

    // P2-1 — datasource-creation DoS cap. Bound the number of UNAUTHENTICATED dynamic datasources per
    // chain key so a counterfeit contract spamming `OutboxCreated` cannot make us register unbounded
    // datasources. The count is a by-id entity (keyed by chain key) so `.get()` sees same-block
    // buffered writes — a `getByField` count would miss datasources created earlier in the same block
    // and let a single-transaction flood bypass the cap entirely. Trusted (authenticated) factories
    // are neither counted nor capped, so a legitimate factory is never blocked by counterfeit rows.
    if (!authenticated) {
        const counter = await OutboxDatasourceCount.get(chainKey);
        const current = counter?.count ?? 0;
        if (current >= MAX_OUTBOXES_PER_CHAIN_KEY) {
            logger.warn(
                `OutboxCreated for ${address}: chainKey ${chainKey} already has ${current} unauthenticated ` +
                    `Outbox datasources (cap ${MAX_OUTBOXES_PER_CHAIN_KEY}) — skipping to bound datasource-creation DoS`,
            );
            return;
        }
    }

    // Persist the parent OutboxContract entity BEFORE spinning up the dynamic datasource. A dynamic
    // datasource created mid-block handles the *same* block's later events, so a `MessagePublished`
    // emitted in the same block as `OutboxCreated` would run its handler — which sets the required
    // `outbox` relation — and must find the parent already staged in the store cache. Saving first
    // guarantees that (both writes commit together in the block's transaction).
    //
    // Ordering note (reconciles three review passes on these lines): SubQuery buffers `.save()` into
    // the per-block store transaction, so save-first does NOT strand the datasource on a partial
    // failure — if `createOutboxDatasource` throws, the whole block rolls back (including this save)
    // and the retry re-runs both cleanly. The residual duplicate-on-crash window is identical in
    // either order (the datasource-metadata write is the only non-transactional step) and is a
    // framework limitation, not an ordering bug; the `OutboxContract.get` guard above covers the
    // common replay/reorg case. `{ address }` binds the template instance; SubQuery sets the
    // datasource's start block to the current block automatically.
    const outbox = OutboxContract.create({
        id: address,
        chainKey,
        factoryId,
        createdAt: BigInt(event.blockNumber),
        createdTimestamp: eventTimestamp(event),
        createdTxHash: event.transactionHash,
    });
    await outbox.save();

    // Charge the per-chain-key DoS counter for unauthenticated datasources only (see the cap above).
    // Read-modify-write by id so multiple OutboxCreated in one block increment correctly (by-id gets
    // see same-block buffered writes).
    if (!authenticated) {
        const counter = await OutboxDatasourceCount.get(chainKey);
        await OutboxDatasourceCount.create({ id: chainKey, count: (counter?.count ?? 0) + 1 }).save();
    }

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

    const [messageIdRaw, emitterRaw, requiresAck, payload] = event.args;
    const messageId = messageIdRaw;
    // emitterAddress is now a bytes32 with the 20-byte EVM address in the high bytes
    // (bytes32(bytes20(emitter))). Recover the plain address so stored/queried emitters stay
    // 20-byte addresses, consistent with the rest of the schema.
    const emitter = `0x${emitterRaw.slice(2, 42)}`.toLowerCase();
    logger.info(`MessagePublished: messageId=${messageId}, emitter=${emitter}, requiresAck=${requiresAck}`);

    // Idempotency guard: a replayed MessagePublished (reorg replay, reindex overlap, duplicate
    // datasource) must not reset a message that handleMessageAcknowledged already marked
    // acknowledged — the publish fields are immutable per messageId, so there is nothing to
    // update either. Skip instead of overwriting.
    const existing = await OutboxMessage.get(messageId);
    if (existing) {
        logger.warn(`MessagePublished replay for already-indexed message ${messageId} — keeping existing record`);
        return;
    }

    // Keyed by messageId so handleMessageAcknowledged can load-and-update the same record.
    // outboxId references the OutboxContract created by handleOutboxCreated (same lowercased address).
    const message = OutboxMessage.create({
        id: messageId,
        outboxId: event.address.toLowerCase(),
        emitter,
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
