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
import {
    authorizedOutboxChainKey,
    discoveryAddress,
    isAuthorizedOutbox,
    registeredOutboxes,
} from './outboxAuthorization';

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
// OutboxCreated(address indexed outbox, uint32 indexed chainKey, address indexed owner, address validator, string version)
type OutboxCreatedArgs = [string, bigint, string, string, string];
// OutboxDiscovery: OutboxRegistered(uint32 indexed chainKey, address indexed outbox, address indexed registrar)
type OutboxRegisteredArgs = [bigint, string, string];
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

// Candidate bounds and the old message-quarantine cleanup limit. Authentic registry membership
// bypasses this candidate cap; permissionless factory deployments cannot exhaust admission.
// Both values are getByFields limits, which SubQuery hard-caps at 100.
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

    // A registered factory is permissionless: its event proves provenance, not authorization.
    // Admission additionally requires membership in governance's Discovery at this indexed block.
    const registration = await OutboxFactoryRegistration.get(chainKeyNumber.toString());
    if (
        factoryId &&
        registration &&
        registration.factoryAddress === factoryId &&
        (await isAuthorizedOutbox(chainKeyNumber, address, event.blockNumber))
    ) {
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

    // Keep only a bounded candidate announcement. A later authentic Discovery registration or
    // an authorized publication may admit it; factory registration alone never does. Messages
    // published before Discovery authorization are not backfilled as authorized history.
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
                `Discovery registration will admit legitimate Outboxes independently of this candidate cap.`,
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
    factoryAddress?: string;
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

/** Refresh bounded candidates after factory registration; Discovery remains authoritative. */
export async function promotePendingOutboxes(
    chainKey: bigint,
    factoryAddress: string,
    blockNumber: number,
): Promise<void> {
    await flushStore();
    const pending = await PendingOutbox.getByFields([['chainKey', '=', chainKey]], {
        limit: MAX_PENDING_OUTBOXES_PER_CHAIN_KEY,
    });
    for (const p of pending) {
        if (p.factoryAddress !== factoryAddress || !(await isAuthorizedOutbox(chainKey, p.id, blockNumber))) continue;
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
        await discardPendingOutbox(p.id);
    }
}

/** Retire old quarantine rows without retroactively authorizing publications. */
async function discardPendingOutbox(address: string): Promise<void> {
    await flushStore();
    const messages = await QuarantinedMessage.getByFields([['outboxAddress', '=', address]], {
        limit: MAX_QUARANTINED_MESSAGES_PER_OUTBOX,
    });
    for (const message of messages) await QuarantinedMessage.remove(message.id);
    await PendingOutbox.remove(address);
}

/** The caller has verified this address against the canonical registry at the indexed height. */
async function admitRegistryMember(
    address: string,
    chainKey: bigint,
    blockNumber: number,
    timestamp: bigint,
    txHash: string,
): Promise<void> {
    if (!(await OutboxContract.get(address))) {
        // Do not trust metadata from arbitrary OutboxCreated emitters. The registry event/snapshot
        // suffices for admission even if the factory event was earlier or the candidate cap is full.
        await admitOutbox({
            address,
            chainKeyBytes32: u64ChainKeyToBytes32(chainKey),
            createdAt: BigInt(blockNumber),
            createdTimestamp: timestamp,
            createdTxHash: txHash,
        });
    }
    await discardPendingOutbox(address);
}

export async function handleOutboxRegistered(event: FrontierEvmEvent<OutboxRegisteredArgs>): Promise<void> {
    if (!event.args || !event.address || !event.transactionHash) return;
    const [chainKeyRaw, outboxRaw] = event.args;
    const chainKey = BigInt(chainKeyRaw);
    const address = outboxRaw.toLowerCase();
    if ((await discoveryAddress(chainKey)) !== event.address.toLowerCase()) return;
    if (!(await isAuthorizedOutbox(chainKey, address, event.blockNumber))) return;
    await admitRegistryMember(address, chainKey, event.blockNumber, eventTimestamp(event), event.transactionHash);
}

/** Bootstrap a newly governance-registered Discovery, including Outboxes registered before it. */
export async function admitDiscoverySnapshot(
    chainKey: bigint,
    discovery: string,
    blockNumber: number,
    timestamp: bigint,
    txHash: string,
): Promise<void> {
    for (const address of await registeredOutboxes(chainKey, discovery, blockNumber)) {
        await admitRegistryMember(address, chainKey, blockNumber, timestamp, txHash);
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

    // Authorization is checked at publication height every time. An existing history row must
    // not keep admitting new messages after a scheduled removal, registry rotation or chain removal.
    // Pending creation events only identify candidates; never promote unauthorized past messages.
    const outbox = await OutboxContract.get(outboxAddress);
    // PendingOutbox is deliberately not a routing authority: an attacker can announce a real
    // address under the wrong key or fill the candidate cap. Query all configured registries
    // for a previously unknown emitter, including publication before registration in this block.
    const chainKey = outbox
        ? BigInt(outbox.chainKey)
        : await authorizedOutboxChainKey(outboxAddress, event.blockNumber);
    if (chainKey === undefined) return;
    if (outbox && !(await isAuthorizedOutbox(chainKey, outboxAddress, event.blockNumber))) return;
    if (!outbox) {
        await admitRegistryMember(
            outboxAddress,
            chainKey,
            event.blockNumber,
            eventTimestamp(event),
            event.transactionHash,
        );
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

    // No pre-authorization publications are retained or promoted. A previously admitted message
    // may still receive its same-emitter acknowledgement after its Outbox leaves the active set.
    logger.debug(`MessageAcknowledged for unknown message ${messageId} from ${outboxAddress} — skipping`);
}
