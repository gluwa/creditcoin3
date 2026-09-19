import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import {
    OutboxContract,
    OutboxFactoryRegistration,
    OutboxMessage,
    PendingOutbox,
    QuarantinedMessage,
    SupportedChain,
    TransactionVerified,
} from '../types';
import { flushStore } from './storeUtils';

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

// USC write-ability: on-chain-discovered Outbox contracts on Creditcoin L1, authorized per event
// against governance state (see datasources.ts for the discovery/authorization model).
// OutboxFactory: OutboxCreated(bytes32 indexed chainKey, address indexed outboxAddress)
// OutboxCreated(address indexed outbox, uint32 indexed chainKey, address indexed owner, address validator, string version)
type OutboxCreatedArgs = [string, bigint, string, string, string];
// Outbox: MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool canAck, bytes payload)
// canAck (renamed from requiresAck in usc-contracts #23): acknowledgment is optional, requested by
// a nonzero acknowledgmentPrice in the signed relayer quote — the flag only says an ack MAY land.
// emitterAddress is a bytes32 (20-byte EVM address left-aligned in the high bytes).
type MessagePublishedArgs = [string, string, boolean, string];
// Outbox: MessageAcknowledged(bytes32 indexed messageId)
type MessageAcknowledgedArgs = [string];

function eventTimestamp(event: { blockTimestamp?: Date }): bigint {
    return event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now());
}

// Quarantine bounds. Both caps are deliberately small: in a correct deployment the quarantine only
// bridges the gap between `deployOutbox` and its registration being indexed (a few blocks), so
// anything approaching these numbers is counterfeit traffic. Rejected overflow is logged loudly —
// a legitimate Outbox hitting the cap is an operator problem to surface, never silent truncation.
// Both values double as `getByFields` limits, which SubQuery hard-caps at 100 — a larger cap makes
// the lookup THROW at runtime, halting the indexer on the first quarantined message (seen in CI).
export const MAX_PENDING_OUTBOXES_PER_CHAIN_KEY = 8;
export const MAX_QUARANTINED_MESSAGES_PER_OUTBOX = 100;

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
    const chainKeyNumber = BigInt(chainKeyRaw);
    const chainKey = u64ChainKeyToBytes32(chainKeyNumber);
    const address = outboxAddress.toLowerCase();
    // event.address is the factory that emitted OutboxCreated.
    const factoryId = event.address ? event.address.toLowerCase() : undefined;

    logger.info(`OutboxCreated: outbox=${address}, chainKey=${chainKey}, factory=${factoryId}`);

    // Idempotency guard: reprocessing the same OutboxCreated log (reorg replay, reindex overlap)
    // must not disturb an already-admitted Outbox.
    const existing = await OutboxContract.get(address);
    if (existing) {
        logger.warn(`OutboxCreated for already-admitted outbox ${address} — keeping existing record`);
        return;
    }

    // An OutboxContract row is the authorization the chain-wide message handlers key on, so creating
    // one requires the emitting factory to be the exact address governance registered for this raw
    // USC chain key. `OutboxFactoryRegistration` is keyed by chain key, which also handles multi-key
    // factories and rotations without relying on the display-oriented OutboxFactory row.
    const registration = await OutboxFactoryRegistration.get(chainKeyNumber.toString());
    if (factoryId && registration && registration.factoryAddress === factoryId) {
        await admitOutbox({
            address,
            chainKeyBytes32: chainKey,
            factoryAddress: factoryId,
            createdAt: BigInt(event.blockNumber),
            createdTimestamp: eventTimestamp(event),
            createdTxHash: event.transactionHash,
        });
        return;
    }

    // Fail-closed, but not fail-forever: the registration for this chain key may simply not be
    // indexed yet (governance can authorize a factory AFTER it deployed its Outbox — observed live
    // on usc-dev, where OutboxCreated landed ~200 blocks before OutboxFactoryRegistered). Quarantine
    // the announcement so handleOutboxFactoryRegistered can promote it retroactively. Quarantine is
    // bounded: only chain keys governance actually created can hold pending rows, and each key holds
    // at most MAX_PENDING_OUTBOXES_PER_CHAIN_KEY of them.
    if (!factoryId) {
        logger.warn(`Rejecting OutboxCreated with no emitter address: outbox=${address}`);
        return;
    }
    if (await PendingOutbox.get(address)) {
        logger.warn(`OutboxCreated replay for already-quarantined outbox ${address} — keeping existing record`);
        return;
    }
    // Same-block writes are only visible to getByFields after a flush.
    await flushStore();
    const knownChain = await SupportedChain.getByFields([['chainKey', '=', chainKeyNumber]], { limit: 1 });
    if (knownChain.length === 0) {
        logger.warn(
            `Rejecting OutboxCreated for unknown chain key: outbox=${address}, chainKey=${chainKeyNumber.toString()}, ` +
                `emitter=${factoryId} (chain key was never registered — not quarantining)`,
        );
        return;
    }
    const pendingForKey = await PendingOutbox.getByFields([['chainKey', '=', chainKeyNumber]], {
        limit: MAX_PENDING_OUTBOXES_PER_CHAIN_KEY,
    });
    if (pendingForKey.length >= MAX_PENDING_OUTBOXES_PER_CHAIN_KEY) {
        logger.error(
            `Pending-Outbox quarantine full for chain key ${chainKeyNumber.toString()} ` +
                `(${MAX_PENDING_OUTBOXES_PER_CHAIN_KEY} rows) — dropping OutboxCreated for ${address}. ` +
                `If this Outbox is legitimate, register its factory and reindex past this block.`,
        );
        return;
    }
    logger.info(
        `Quarantining unauthenticated OutboxCreated: outbox=${address}, chainKey=${chainKeyNumber.toString()}, ` +
            `emitter=${factoryId}, registered=${registration?.factoryAddress ?? 'none'}`,
    );
    await PendingOutbox.create({
        id: address,
        chainKey: chainKeyNumber,
        factoryAddress: factoryId,
        chainKeyBytes32: chainKey,
        createdAt: BigInt(event.blockNumber),
        createdTimestamp: eventTimestamp(event),
        createdTxHash: event.transactionHash,
    }).save();
}

/** Create the admitted OutboxContract row — shared by direct admission and quarantine promotion. */
async function admitOutbox(outbox: {
    address: string;
    chainKeyBytes32: string;
    factoryAddress: string;
    createdAt: bigint;
    createdTimestamp: bigint;
    createdTxHash: string;
}): Promise<void> {
    await OutboxContract.create({
        id: outbox.address,
        chainKey: outbox.chainKeyBytes32,
        factoryId: outbox.factoryAddress,
        createdAt: outbox.createdAt,
        createdTimestamp: outbox.createdTimestamp,
        createdTxHash: outbox.createdTxHash,
    }).save();
}

/**
 * Backfill half of fail-closed discovery, called by `handleOutboxFactoryRegistered` right after it
 * stores the registration: promote every quarantined Outbox this registration retroactively
 * authorizes, along with the messages observed on it while it was pending. Non-matching pending rows
 * for the key are left alone — a later rotation may authorize them.
 */
export async function promotePendingOutboxes(chainKey: bigint, factoryAddress: string): Promise<void> {
    // The registration (and, same-block, possibly the pending rows) must be visible to getByFields.
    await flushStore();
    const pending = await PendingOutbox.getByFields([['chainKey', '=', chainKey]], {
        limit: MAX_PENDING_OUTBOXES_PER_CHAIN_KEY,
    });
    for (const p of pending) {
        if (p.factoryAddress !== factoryAddress) {
            continue;
        }
        logger.info(
            `Promoting quarantined Outbox ${p.id} (chainKey=${chainKey.toString()}) — ` +
                `its factory ${factoryAddress} is now governance-registered`,
        );
        if (!(await OutboxContract.get(p.id))) {
            await admitOutbox({
                address: p.id,
                chainKeyBytes32: p.chainKeyBytes32,
                factoryAddress,
                createdAt: p.createdAt,
                createdTimestamp: p.createdTimestamp,
                createdTxHash: p.createdTxHash,
            });
        }
        const messages = await QuarantinedMessage.getByFields([['outboxAddress', '=', p.id]], {
            limit: MAX_QUARANTINED_MESSAGES_PER_OUTBOX,
        });
        for (const m of messages) {
            if (!(await OutboxMessage.get(m.id))) {
                await OutboxMessage.create({
                    id: m.id,
                    outboxId: p.id,
                    emitter: m.emitter,
                    canAck: m.canAck,
                    payload: m.payload,
                    publishedAt: m.publishedAt,
                    publishedTimestamp: m.publishedTimestamp,
                    publishedTxHash: m.publishedTxHash,
                    acknowledged: m.acknowledged,
                    acknowledgedAt: m.acknowledgedAt,
                    acknowledgedTimestamp: m.acknowledgedTimestamp,
                    acknowledgedTxHash: m.acknowledgedTxHash,
                }).save();
            }
            await QuarantinedMessage.remove(m.id);
        }
        if (messages.length > 0) {
            logger.info(`Backfilled ${messages.length} quarantined message(s) for promoted Outbox ${p.id}`);
        }
        await PendingOutbox.remove(p.id);
    }
}

/**
 * Drop every quarantined Outbox (and its quarantined messages) for a chain key governance removed —
 * called by `handleSupportedChainRemoved` after it revokes the factory registration. Without a chain
 * key there is nothing left that could ever authorize these rows.
 */
export async function purgePendingOutboxes(chainKey: bigint): Promise<void> {
    await flushStore();
    const pending = await PendingOutbox.getByFields([['chainKey', '=', chainKey]], {
        limit: MAX_PENDING_OUTBOXES_PER_CHAIN_KEY,
    });
    for (const p of pending) {
        const messages = await QuarantinedMessage.getByFields([['outboxAddress', '=', p.id]], {
            limit: MAX_QUARANTINED_MESSAGES_PER_OUTBOX,
        });
        for (const m of messages) {
            await QuarantinedMessage.remove(m.id);
        }
        await PendingOutbox.remove(p.id);
        logger.info(`Purged quarantined Outbox ${p.id} (chain key ${chainKey.toString()} removed)`);
    }
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

    const [messageIdRaw, emitterRaw, canAck, payload] = event.args;
    const messageId = messageIdRaw;
    // emitterAddress is now a bytes32 with the 20-byte EVM address in the high bytes
    // (bytes32(bytes20(emitter))). Recover the plain address so stored/queried emitters stay
    // 20-byte addresses, consistent with the rest of the schema.
    const emitter = `0x${emitterRaw.slice(2, 42)}`.toLowerCase();
    const outboxAddress = event.address.toLowerCase();

    // Per-event authorization: this handler is chain-wide (no address filter), so the emitting
    // contract decides the event's fate. An admitted Outbox indexes normally; one still in
    // quarantine gets its message quarantined alongside it (promoted together later); anything
    // else is an arbitrary contract emitting a look-alike event and is dropped without state.
    const outbox = await OutboxContract.get(outboxAddress);
    if (!outbox) {
        const pending = await PendingOutbox.get(outboxAddress);
        if (!pending) {
            logger.debug(`Ignoring MessagePublished from ${outboxAddress} — not an admitted or pending Outbox`);
            return;
        }
        await quarantineMessage(event, event.transactionHash, messageId, outboxAddress, emitter, canAck, payload);
        return;
    }

    logger.info(`MessagePublished: messageId=${messageId}, emitter=${emitter}, canAck=${canAck}`);

    // Idempotency guard: a replayed MessagePublished (reorg replay, reindex overlap) must not reset
    // a message that handleMessageAcknowledged already marked acknowledged — the publish fields are
    // immutable per messageId, so there is nothing to update either. Skip instead of overwriting.
    const existing = await OutboxMessage.get(messageId);
    if (existing) {
        logger.warn(`MessagePublished replay for already-indexed message ${messageId} — keeping existing record`);
        return;
    }

    // Keyed by messageId so handleMessageAcknowledged can load-and-update the same record.
    // outboxId references the OutboxContract created by handleOutboxCreated (same lowercased address).
    const message = OutboxMessage.create({
        id: messageId,
        outboxId: outboxAddress,
        emitter,
        canAck,
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

/** Hold a message observed on a still-pending Outbox for promotion, bounded per Outbox. */
async function quarantineMessage(
    event: FrontierEvmEvent<MessagePublishedArgs>,
    // Separate from `event` because the caller has already null-guarded it (TS can't carry that
    // narrowing across the function boundary).
    publishedTxHash: string,
    messageId: string,
    outboxAddress: string,
    emitter: string,
    canAck: boolean,
    payload: string,
): Promise<void> {
    if (await QuarantinedMessage.get(messageId)) {
        logger.warn(`MessagePublished replay for already-quarantined message ${messageId} — keeping existing record`);
        return;
    }
    await flushStore();
    const held = await QuarantinedMessage.getByFields([['outboxAddress', '=', outboxAddress]], {
        limit: MAX_QUARANTINED_MESSAGES_PER_OUTBOX,
    });
    if (held.length >= MAX_QUARANTINED_MESSAGES_PER_OUTBOX) {
        logger.error(
            `Message quarantine full for pending Outbox ${outboxAddress} ` +
                `(${MAX_QUARANTINED_MESSAGES_PER_OUTBOX} rows) — dropping message ${messageId}. ` +
                `If this Outbox is legitimate, register its factory and reindex past this block.`,
        );
        return;
    }
    logger.info(`Quarantining MessagePublished ${messageId} from pending Outbox ${outboxAddress}`);
    await QuarantinedMessage.create({
        id: messageId,
        outboxAddress,
        emitter,
        canAck,
        payload,
        publishedAt: BigInt(event.blockNumber),
        publishedTimestamp: eventTimestamp(event),
        publishedTxHash,
        acknowledged: false,
        acknowledgedAt: undefined,
        acknowledgedTimestamp: undefined,
        acknowledgedTxHash: undefined,
    }).save();
}

export async function handleMessageAcknowledged(event: FrontierEvmEvent<MessageAcknowledgedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for MessageAcknowledged event at block ${event.blockNumber}`);
        return;
    }

    const [messageId] = event.args;
    if (!event.address) {
        logger.error(`Contract address missing for MessageAcknowledged at block ${event.blockNumber}. Skipping.`);
        return;
    }
    const outboxAddress = event.address.toLowerCase();

    // Per-event authorization, like handleMessagePublished: this handler is chain-wide, so an ack is
    // only honored when it was emitted by the same contract that holds the message. Without this, any
    // contract could emit MessageAcknowledged with a known messageId and flip a real message's state.
    const message = await OutboxMessage.get(messageId);
    if (message) {
        if (message.outboxId !== outboxAddress) {
            logger.warn(
                `Ignoring MessageAcknowledged for ${messageId} from ${outboxAddress} — ` +
                    `the message belongs to Outbox ${message.outboxId}`,
            );
            return;
        }
        logger.info(`MessageAcknowledged: messageId=${messageId}`);
        message.acknowledged = true;
        message.acknowledgedAt = BigInt(event.blockNumber);
        message.acknowledgedTimestamp = eventTimestamp(event);
        message.acknowledgedTxHash = event.transactionHash ?? undefined;
        await message.save();
        return;
    }

    // An ack can land while the message is still quarantined (Outbox not yet authorized). Record it
    // on the quarantine row so promotion carries the full lifecycle, not just the publish.
    const quarantined = await QuarantinedMessage.get(messageId);
    if (quarantined) {
        if (quarantined.outboxAddress !== outboxAddress) {
            logger.warn(
                `Ignoring MessageAcknowledged for quarantined ${messageId} from ${outboxAddress} — ` +
                    `the message was observed on ${quarantined.outboxAddress}`,
            );
            return;
        }
        logger.info(`MessageAcknowledged (quarantined): messageId=${messageId}`);
        quarantined.acknowledged = true;
        quarantined.acknowledgedAt = BigInt(event.blockNumber);
        quarantined.acknowledgedTimestamp = eventTimestamp(event);
        quarantined.acknowledgedTxHash = event.transactionHash ?? undefined;
        await quarantined.save();
        return;
    }

    // The publish is always seen first when its Outbox is admitted or pending. If neither record
    // exists, this is either an arbitrary contract emitting a look-alike event (drop silently-ish)
    // or an Outbox that was never authorized at all.
    logger.debug(`MessageAcknowledged for unknown message ${messageId} from ${outboxAddress} — skipping`);
}
