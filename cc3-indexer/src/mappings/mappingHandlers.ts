import assert from 'assert';
import { SubstrateEvent, SubstrateExtrinsic } from '@subql/types';
import {
    AttestorsElected,
    AttestorRegistered,
    AttestorUnregistered,
    InvulnerableRegistered,
    InvulnerableUnregistered,
    TargetSampleSizeChanged,
    PendingTargetSampleSizeSet,
    Bonded,
    Unbonded,
    Withdrawn,
    AttestorActivated,
    AttestorChilled,
    MinBondRequirementUpdated,
    CheckpointsCleared,
    ClearedStorageForRemovedChain,
    AttestationIntervalChanged,
    PendingAttestationIntervalSet,
    Attestors,
    Checkpoints,
    Attestations,
    MapAttestationAttestor,
    CheckpointIntervalChanged,
    ChainRegistered,
    SupportedChain,
    ChainRemoved,
    MaturityStrategySet,
    AttestationChainData,
    MaxAttestorsChanged,
    VoteAcceptanceWindowChanged,
    ChangedElectionPolicy,
    AuthorizedAttestorAdded,
    AuthorizedAttestors,
    AuthorizedAttestorRemoved,
    ContinuityProof,
} from '../types';
import { Balance } from '@polkadot/types/interfaces';
import { getChainData } from './initStore';

export async function handleEventAttestorsElected(event: SubstrateEvent): Promise<void> {
    logger.info(`New Attestors Elected event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [epoch, chainKey, attestors],
        },
    } = event;

    const chainKeyStr = chainKey.toString();
    const epochStr = epoch.toString();
    const chainKeyNumber = BigInt(chainKeyStr);
    const epochNumber = parseInt(epochStr, 10);
    const blockNumber = event.block.block.header.number.toBigInt();

    const saveEntityList = [];

    if (Array.isArray(attestors)) {
        for (let index = 0; index < attestors.length; index++) {
            const account = attestors[index];
            console.log('Processing account:', account);

            const accountStr = account.toString();

            const attestorsElected = AttestorsElected.create({
                id: `${blockNumber}-${event.idx}-${index}`,
                epoch: BigInt(epochNumber),
                chainKey: chainKeyNumber,
                attestorId: accountStr,
            });

            saveEntityList.push(attestorsElected.save());

            const id = `${blockNumber}-${event.idx}-${index}`;
            const attestorEntity = await checkAndGetAttestor(id, accountStr, chainKeyNumber);
            attestorEntity.lastUpdateBlockNumber = event.block.block.header.number.toBigInt();
            attestorEntity.status = 3; // 3 - Active

            saveEntityList.push(attestorEntity.save());
        }
    } else {
        logger.error(`Attestors is not a valid at: ${blockNumber}`);
    }

    try {
        await Promise.all(saveEntityList);
        logger.info(`All attestors have been dynamically added and saved at block: ${blockNumber}`);
    } catch (_error) {
        logger.error(`An error occurred while saving attestorsElected at block: ${blockNumber}`);
    }
}

export async function handleSupportedChainRegistered(event: SubstrateEvent): Promise<void> {
    const {
        event: {
            data: [chainKey, chainId, chainName, chainEncoding, maturityStrategy],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const hexDecodedChainName = Buffer.from(chainName.toString().slice(2), 'hex').toString('utf8');
    const chainEncodingStr = chainEncoding.toString();
    const maturityStrategyStr = maturityStrategy.toString();

    const chainRegistered = ChainRegistered.create({
        id: `${blockNumber}-${event.idx}`,
        at: blockNumber,
        chainKey: chainKeyNumber,
        chainName: hexDecodedChainName,
        chainEncoding: chainEncodingStr,
        maturityStrategy: maturityStrategyStr,
        chainId: BigInt(chainId.toString()),
        whoId: from.toString(),
    });

    const suportedChain = SupportedChain.create({
        id: chainKeyNumber.toString(),
        at: blockNumber,
        chainKey: chainKeyNumber,
        chainName: hexDecodedChainName,
        chainEncoding: chainEncodingStr,
        maturityStrategy: maturityStrategyStr,
        chainId: BigInt(chainId.toString()),
    });

    // default to OpenToAny on registration, see pallet attestation for details
    const defaultElectionPolicy = 'OpenToAny';

    // Create attestation chain data
    const newChain = AttestationChainData.create({
        id: chainKeyNumber.toString(),
        chainKey: chainKeyNumber,
        chainReward: BigInt(0),
        attestationInterval: BigInt(10),
        checkpointInterval: 10,
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: BigInt(0),
        lastCheckpointHeaderNumber: BigInt(0),
        maxSetSize: 100,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100000000000000000000),
        voteAcceptanceWindow: BigInt(3),
        electionPolicy: defaultElectionPolicy,
    });

    logger.info(`New Supported Chain event created at block ${blockNumber}`);

    await Promise.all([chainRegistered.save(), suportedChain.save(), newChain.save()]);
}

export async function handleSupportedChainRemoved(event: SubstrateEvent): Promise<void> {
    const {
        event: {
            data: [chainKey, chainId, chainName, chainEncoding, maturityStrategy],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const hexDecodedChainName = Buffer.from(chainName.toString().slice(2), 'hex').toString('utf8');
    const chainEncodingStr = chainEncoding.toString();
    const maturityStrategyStr = maturityStrategy.toString();

    const chainRemoved = ChainRemoved.create({
        id: `${blockNumber}-${event.idx}`,
        at: blockNumber,
        chainKey: chainKeyNumber,
        chainName: hexDecodedChainName,
        chainEncoding: chainEncodingStr,
        maturityStrategy: maturityStrategyStr,
        chainId: BigInt(chainId.toString()),
        whoId: from.toString(),
    });
    await chainRemoved.save();

    const supportedChain = await SupportedChain.getByFields([['chainKey', '=', chainKeyStr]], { limit: 1 });
    if (isEmpty(supportedChain)) {
        logger.error(`Supported Chains : ${chainKeyStr} not found in db for block number event: ${blockNumber}.`);
    } else {
        logger.info(
            `Supported Chains : ${chainKeyStr} found in db for block number event: ${blockNumber}. Supported chain will be removed`,
        );
        await SupportedChain.remove(supportedChain[0].id);
    }

    // Remove attestationChainData
    const attestationChainData = await AttestationChainData.getByFields([['chainKey', '=', chainKeyStr]], { limit: 1 });
    if (isEmpty(attestationChainData)) {
        logger.error(`AttestationChainData : ${chainKeyStr} not found in db for block number event: ${blockNumber}.`);
    } else {
        logger.info(
            `AttestationChainData : ${chainKeyStr} found in db for block number event: ${blockNumber}. Attestation chain data will be removed`,
        );
        await AttestationChainData.remove(attestationChainData[0].id);
    }

    return Promise.resolve();
}

export async function handleMaturityStrategySet(event: SubstrateEvent): Promise<void> {
    const {
        event: {
            data: [chainKey, chainId, chainName, maturityStrategy],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const hexDecodedChainName = Buffer.from(chainName.toString().slice(2), 'hex').toString('utf8');
    const maturityStrategyStr = maturityStrategy.toString();

    const maturityStrategySet = MaturityStrategySet.create({
        id: `${blockNumber}-${event.idx}`,
        at: blockNumber,
        chainKey: chainKeyNumber,
        chainName: hexDecodedChainName,
        maturityStrategy: maturityStrategyStr,
        chainId: BigInt(chainId.toString()),
        whoId: from.toString(),
    });

    const supportedChain = await SupportedChain.get(chainKeyStr);
    if (!supportedChain) {
        logger.error(`Supported Chain : ${chainKeyStr} not found in db for set maturity strategy event: ${event.idx}.`);
    } else {
        supportedChain.maturityStrategy = maturityStrategyStr;
        logger.info(`New MaturityStrategySet event created at block ${blockNumber}`);
        await Promise.all([maturityStrategySet.save(), supportedChain.save()]);
    }
}

export async function handleEventAttestorRegistered(event: SubstrateEvent): Promise<void> {
    logger.info(`New Attestor Registered event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const attestorRegistered = AttestorRegistered.create({
        id: `${blockNumber}-${event.idx}`,
        stashId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    const id = `${blockNumber}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.stashId = from.toString();
    attestorEntity.status = 1; // 1 - Idle

    logger.info(`New AttestorEntity event created at block ${blockNumber}`);

    await Promise.all([attestorRegistered.save(), attestorEntity.save()]);
}

export async function handleEventAttestorUnregistered(event: SubstrateEvent): Promise<void> {
    logger.info(`New Attestor Unregistered event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const attestorUnregistered = AttestorUnregistered.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    const id = `${blockNumber}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 0; // Not registered

    await Promise.all([attestorUnregistered.save(), attestorEntity.save()]);
}

export async function handleEventInvulnerableRegistered(event: SubstrateEvent): Promise<void> {
    logger.info(`New Invulnerable Registered event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const invulnerableRegistered = InvulnerableRegistered.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    await invulnerableRegistered.save();
}

export async function handleEventInvulnerableUnregistered(event: SubstrateEvent): Promise<void> {
    logger.info(`New Invulnerable Unregistered event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const invulnerableUnregistered = InvulnerableUnregistered.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    await invulnerableUnregistered.save();
}

export async function handleEventCheckpointReached(event: SubstrateEvent): Promise<void> {
    logger.info(`New Checkpoint Reached event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestationCheckpoint],
        },
    } = event;

    logger.info(`New Checkpoint Reached ${attestationCheckpoint.toString()}`);

    const checkpoint = parseAttestationCheckpoint(attestationCheckpoint.toString());

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    /* eslint-disable @typescript-eslint/naming-convention */
    const checkpointReached = Checkpoints.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        atBlockNumber: blockNumber,
        chainKey: chainKeyNumber,
        blockNumber: checkpoint.blockNumber,
        digest: checkpoint.digest,
        timestamp: BigInt(event.block.timestamp?.getTime() ?? 0),
    });
    /* eslint-enable */

    const chainData = await getChainData(chainKeyNumber);
    if (chainData) {
        chainData.lastCheckpointHeaderNumber = checkpoint.blockNumber;
        await chainData?.save();
    }

    await checkpointReached.save();
}

export async function handleEventTargetSampleSizeChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`New TargetSampleSizeChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, newTargetSampleSize],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);
    const newTargetSampleSizeNumber = parseInt(newTargetSampleSize.toString(), 10);

    /* eslint-disable @typescript-eslint/naming-convention */
    const targetSampleSizeChanged = TargetSampleSizeChanged.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: '', // empty string - will remove field when migrations are enabled
        blockNumber,
        chainKey: chainKeyNumber,
        eventNewTargetSampleSize: newTargetSampleSizeNumber,
    });
    /* eslint-enable */

    const chainData = await getChainData(chainKeyNumber);
    if (chainData) {
        chainData.targetSampleSize = newTargetSampleSizeNumber;
        await chainData?.save();
    }

    await targetSampleSizeChanged.save();
}

export async function handleEventPendingTargetSampleSizeSet(event: SubstrateEvent): Promise<void> {
    logger.info(`New PendingTargetSampleSizeSet event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, newTargetSampleSize],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const pendingTargetSampleSizeSet = PendingTargetSampleSizeSet.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        targetSampleSize: BigInt(newTargetSampleSize.toString()),
        whoId: from.toString(),
    });

    await pendingTargetSampleSizeSet.save();
}

function isEmpty(value: any): boolean {
    if (value == null) return true; // Checks for null or undefined
    if (typeof value === 'string' || Array.isArray(value)) return value.length === 0;
    if (typeof value === 'object') return Object.keys(value).length === 0;
    return false;
}

async function checkAndGetAttestor(id: string, attestorId: string, chainKey: bigint): Promise<Attestors> {
    const attestor = await Attestors.getByFields(
        [
            ['attestorId', '=', attestorId],
            ['chainKey', '=', chainKey],
        ],
        { limit: 1 },
    );
    if (isEmpty(attestor)) {
        return Attestors.create({
            id: id.toLowerCase(),
            attestorId,
            chainKey,
            lastUpdateBlockNumber: BigInt(0),
            status: 0, // 0 - Not registered, 1 - Idle/Chilled, 2 - Waiting, 3 - Active
            stashId: '',
            blsPublicKey: '',
        });
    }
    return attestor[0];
}

interface AttestationCheckpointData {
    blockNumber: bigint;
    digest: string;
}

function parseAttestationCheckpoint(attestationCheckpointStr: string): AttestationCheckpointData {
    try {
        const parsed: AttestationCheckpointData = JSON.parse(attestationCheckpointStr);

        if (typeof parsed.blockNumber !== 'number' || typeof parsed.digest !== 'string') {
            throw new Error('Invalid AttestationCheckpoint structure');
        }

        return parsed;
    } catch (_error) {
        throw new Error(`Failed to parse AttestationCheckpoint`);
    }
}

export async function handleEventBonded(event: SubstrateEvent): Promise<void> {
    logger.info(`New Bonded event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [stash, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const bonded = Bonded.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as unknown as Balance).toBigInt(),
    });

    await bonded.save();
}

export async function handleEventUnbonded(event: SubstrateEvent): Promise<void> {
    logger.info(`New Unbonded event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [stash, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const unbonded = Unbonded.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as unknown as Balance).toBigInt(),
    });

    await unbonded.save();
}

export async function handleEventWithdrawn(event: SubstrateEvent): Promise<void> {
    logger.info(`New Withdrawn event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [stash, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const withdrawn = Withdrawn.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as unknown as Balance).toBigInt(),
    });

    await withdrawn.save();
}

// Store digest from reduced event to match with call data
const pendingDigests = new Map<string, string>();

export async function handleEventBlockAttested(event: SubstrateEvent): Promise<void> {
    logger.info(`Block Attested event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, headerNumber, digest],
        },
    } = event;

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);
    const headerNumberBigInt = BigInt(headerNumber.toString());
    const digestStr = digest.toString();

    // Store digest for matching with call handler
    const digestKey = `${chainKeyStr}-${headerNumber.toString()}`;
    pendingDigests.set(digestKey, digestStr);

    // Clean up old entries to prevent memory leak (keep only recent entries)
    // Delete entries older than 100 blocks to prevent unbounded growth
    // Note: This is a simple cleanup strategy; entries are also deleted when retrieved
    if (pendingDigests.size > 1000) {
        // If map grows too large, clear oldest entries (simple approach: clear all and let new ones populate)
        // In practice, entries should be cleaned up when retrieved, so this is a safety net
        logger.warn(`pendingDigests map size is ${pendingDigests.size}, clearing old entries`);
        pendingDigests.clear();
    }

    logger.info(
        `Block Attested event: chain_key=${chainKeyStr}, header_number=${headerNumber.toString()}, digest=${digestStr}`,
    );

    const blockNumber = event.block.block.header.number.toBigInt();

    // Try to find the commit_attestation call in the same block
    // The event is emitted by an extrinsic, so check event.extrinsic
    let foundCall = false;
    if (event.extrinsic) {
        const extrinsic = event.extrinsic;
        const section = extrinsic.extrinsic?.method?.section;
        const method = extrinsic.extrinsic?.method?.method;

        logger.info(`Event extrinsic: section=${section}, method=${method} at block ${blockNumber.toString()}`);

        // Method name is in camelCase: commitAttestation (not commit_attestation)
        if (section === 'attestation' && (method === 'commitAttestation' || method === 'commit_attestation')) {
            logger.info(`Found commit_attestation call in same extrinsic as event at block ${blockNumber.toString()}`);
            foundCall = true;
            // Process the call data here - extract from the extrinsic that emitted the event
            await processAttestationFromCall(
                extrinsic,
                chainKeyNumber,
                chainKeyStr,
                headerNumberBigInt,
                digestStr,
                blockNumber,
                event.block,
            );
        } else {
            logger.warn(
                `Event extrinsic is not commit_attestation: ${section}.${method} at block ${blockNumber.toString()}`,
            );
        }
    } else {
        logger.warn(`No extrinsic found for event at block ${blockNumber.toString()}`);
    }

    if (!foundCall) {
        logger.warn(
            `Could not find commit_attestation call in event extrinsic at block ${blockNumber.toString()}, will rely on call handler`,
        );
    }

    // Update chain data with digest from event
    const chainData = await getChainData(chainKeyNumber);
    if (chainData) {
        chainData.lastAttestedHeaderNumber = headerNumberBigInt;
        chainData.lastAttestedDigest = digestStr;
        await chainData?.save();
    }
}

async function processAttestationFromCall(
    extrinsic: any,
    chainKeyNumber: bigint,
    chainKeyStr: string,
    headerNumberBigInt: bigint,
    digestStr: string,
    blockNumber: bigint,
    eventBlock: any,
): Promise<void> {
    try {
        const signedAttestation = extrinsic.extrinsic?.method?.args?.[0];

        if (!signedAttestation) {
            logger.warn(`No attestation found in commit_attestation call at block ${blockNumber}`);
            return;
        }

        const signedAttestationParsed = parseSignedAttestation(
            typeof signedAttestation === 'string' ? signedAttestation : JSON.stringify(signedAttestation),
        );

        logger.info(`Processing attestation from call in event handler: ${JSON.stringify(signedAttestationParsed)}`);

        const headerNumberStr = signedAttestationParsed.attestation.headerNumber.toString();
        const extrinsicIdx = extrinsic.idx !== undefined ? extrinsic.idx : 0;
        const attestationId = `${blockNumber}-${extrinsicIdx}`;

        // Check if attestation already exists (created by call handler) to prevent duplicates
        let blockAttested = await Attestations.get(attestationId);

        if (!blockAttested) {
            blockAttested = Attestations.create({
                id: attestationId,
                chainKey: signedAttestationParsed.attestation.chainKey,
                headerNumber: signedAttestationParsed.attestation.headerNumber,
                headerHash: signedAttestationParsed.attestation.headerHash,
                root: signedAttestationParsed.attestation.root,
                prevDigest: signedAttestationParsed.attestation.prevDigest ?? '',
                signature: signedAttestationParsed.signature,
                digest: digestStr,
                timestamp: BigInt(eventBlock.timestamp?.getTime() ?? Date.now()),
                continuityProof: signedAttestationParsed.continuityProof
                    ? signedAttestationParsed.continuityProof
                    : undefined,
            });
        } else {
            // Update existing attestation with digest if it was missing
            if (digestStr && !blockAttested.digest) {
                blockAttested.digest = digestStr;
            }
        }

        // Clean up digest from map after processing to prevent memory leak
        const digestKey = `${chainKeyStr}-${headerNumberStr}`;
        pendingDigests.delete(digestKey);

        const saveEntityList = [blockAttested.save()];

        for (let index = 0; index < signedAttestationParsed.attestors.length; index++) {
            const id = `${blockNumber}-${extrinsicIdx}-${index}`;
            const attestor = signedAttestationParsed.attestors[index];
            const attestorEntity = await checkAndGetAttestor(id, attestor, chainKeyNumber);

            // Check if MapAttestationAttestor already exists to prevent duplicates
            let blockAttestor = await MapAttestationAttestor.get(id);
            if (!blockAttestor) {
                blockAttestor = MapAttestationAttestor.create({
                    id,
                    attestorId: attestorEntity.id,
                    attestationId,
                });
                saveEntityList.push(blockAttestor.save());
                logger.info(
                    `Saved map for attestor ${attestor} and attestation ${attestationId} at block ${blockNumber}`,
                );
            } else {
                logger.info(`MapAttestationAttestor ${id} already exists, skipping creation`);
            }
        }

        await Promise.all(saveEntityList);
        logger.info(`Attestation processed from call in event handler for ${attestationId} at block ${blockNumber}`);
    } catch (error) {
        logger.error(`Error processing attestation from call in event handler: ${String(error)}`);
    }
}

export async function handleCallCommitAttestation(extrinsic: SubstrateExtrinsic): Promise<void> {
    const blockNumber = extrinsic.block.block.header.number.toBigInt();
    const section = extrinsic.extrinsic.method.section;
    const method = extrinsic.extrinsic.method.method;

    logger.info(
        `Commit Attestation call handler invoked at block ${blockNumber.toString()}, section=${section}, method=${method}, success=${extrinsic.success}`,
    );

    // Only process successful calls
    if (!extrinsic.success) {
        logger.warn(`Commit Attestation call failed at block ${blockNumber.toString()}, skipping`);
        return;
    }

    logger.info(`Commit Attestation call found at block ${blockNumber.toString()}`);

    // Log call details for debugging
    logger.info(`Call method: ${section}.${method}`);
    logger.info(`Call args length: ${extrinsic.extrinsic.method.args.length}`);

    // Extract single attestation from call arguments
    const signedAttestation = extrinsic.extrinsic.method.args[0];

    if (!signedAttestation) {
        logger.warn(
            `No attestation found in commit_attestation call at block ${blockNumber}, args: ${JSON.stringify(extrinsic.extrinsic.method.args)}`,
        );
        return;
    }

    try {
        const signedAttestationParsed = parseSignedAttestation(
            typeof signedAttestation === 'string' ? signedAttestation : JSON.stringify(signedAttestation),
        );

        logger.info(`Processing attestation from call: ${JSON.stringify(signedAttestationParsed)}`);

        const chainKeyNumber = BigInt(signedAttestationParsed.attestation.chainKey.toString());
        const chainKeyStr = signedAttestationParsed.attestation.chainKey.toString();
        const headerNumberStr = signedAttestationParsed.attestation.headerNumber.toString();

        // Try to get digest from pending digests (set by event handler)
        const digestKey = `${chainKeyStr}-${headerNumberStr}`;
        const digest = pendingDigests.get(digestKey) || '';

        // Clean up digest from map after retrieving it to prevent memory leak
        if (digest) {
            pendingDigests.delete(digestKey);
        }

        // If digest not found, log warning but continue (it might be set later by event)
        if (!digest) {
            logger.warn(`Digest not found for ${digestKey}, will be updated when event is processed`);
        }

        // Use consistent extrinsic.idx handling with fallback to 0
        const extrinsicIdx = extrinsic.idx !== undefined ? extrinsic.idx : 0;
        const attestationId = `${blockNumber}-${extrinsicIdx}`;

        // Check if attestation already exists to prevent duplicates
        let blockAttested = await Attestations.get(attestationId);
        const digestWasEmpty = !digest;

        if (!blockAttested) {
            blockAttested = Attestations.create({
                id: attestationId,
                chainKey: signedAttestationParsed.attestation.chainKey,
                headerNumber: signedAttestationParsed.attestation.headerNumber,
                headerHash: signedAttestationParsed.attestation.headerHash,
                root: signedAttestationParsed.attestation.root,
                prevDigest: signedAttestationParsed.attestation.prevDigest ?? '',
                signature: signedAttestationParsed.signature,
                digest,
                timestamp: BigInt(extrinsic.block.timestamp?.getTime() ?? 0),
                continuityProof: signedAttestationParsed.continuityProof
                    ? signedAttestationParsed.continuityProof
                    : undefined,
            });
        } else {
            // Update existing attestation with digest if it was missing
            if (digest && !blockAttested.digest) {
                blockAttested.digest = digest;
            }
        }

        const saveEntityList = [blockAttested.save()];

        for (let index = 0; index < signedAttestationParsed.attestors.length; index++) {
            const id = `${blockNumber}-${extrinsicIdx}-${index}`;
            const attestor = signedAttestationParsed.attestors[index];
            const attestorEntity = await checkAndGetAttestor(id, attestor, chainKeyNumber);

            // Check if MapAttestationAttestor already exists to prevent duplicates
            let blockAttestor = await MapAttestationAttestor.get(id);
            if (!blockAttestor) {
                blockAttestor = MapAttestationAttestor.create({
                    id,
                    attestorId: attestorEntity.id,
                    attestationId,
                });
                saveEntityList.push(blockAttestor.save());
                logger.info(
                    `Saved map for attestor ${attestor} and attestation ${attestationId} at block ${blockNumber}`,
                );
            } else {
                logger.info(`MapAttestationAttestor ${id} already exists, skipping creation`);
            }
        }

        // Update digest if it was missing initially and we found it later
        if (digest && digestWasEmpty && blockAttested.digest !== digest) {
            blockAttested.digest = digest;
            await blockAttested.save();
        }

        await Promise.all(saveEntityList);
        logger.info(`Commit Attestation call processed for attestation ${attestationId} at block ${blockNumber}`);
    } catch (error) {
        logger.error(`Error processing attestation in commit_attestation call: ${String(error)}`);
    }
}

interface Attestation {
    chainKey: bigint;
    headerNumber: bigint;
    headerHash: string;
    root: string;
    prevDigest: string;
}

interface SignedAttestation {
    attestation: Attestation;
    signature: string;
    attestors: string[];
    continuityProof?: ContinuityProof;
}

function parseSignedAttestation(attestationCheckpointStr: string): SignedAttestation {
    try {
        const parsed: SignedAttestation = JSON.parse(attestationCheckpointStr);

        if (typeof typeof parsed.signature !== 'string') {
            throw new Error('Invalid SignedAttestation structure');
        }

        return parsed;
    } catch (_error) {
        throw new Error(`Failed to parse SignedAttestation`);
    }
}

export async function handleEventAttestorActivated(event: SubstrateEvent): Promise<void> {
    logger.info(`New AttestorActivated event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor, blsPublicKey],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    let blsPublicKeyStr = '';
    if (blsPublicKey) {
        logger.info(`blsPublicKey at block ${blockNumber} is ${blsPublicKey.toString()}`);
        blsPublicKeyStr = blsPublicKey.toString();
    }

    const attestorActivated = AttestorActivated.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
        date: event.block.timestamp,
        blsPublicKey: blsPublicKeyStr,
    });

    const id = `${blockNumber}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 3; // 3 - Became Active
    attestorEntity.blsPublicKey = blsPublicKeyStr;

    await Promise.all([attestorActivated.save(), attestorEntity.save()]);
}

export async function handleEventAttestorChilled(event: SubstrateEvent): Promise<void> {
    logger.info(`New AttestorChilled event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = BigInt(chainKeyStr);

    const attestorChilled = AttestorChilled.create({
        id: `${blockNumber}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
        date: event.block.timestamp,
    });

    const id = `${blockNumber}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 1; // 1 - Chilled/Idle

    await Promise.all([attestorChilled.save(), attestorEntity.save()]);
}

export async function handleEventMinBondRequirementUpdated(event: SubstrateEvent): Promise<void> {
    logger.info(`New MinBondRequirementUpdated event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber = event.block.block.header.number.toBigInt();

    const minBondRequirementUpdated = MinBondRequirementUpdated.create({
        id: `${blockNumber}-${event.idx}`,
        chainKey: BigInt(chainKey.toString()),
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        amount: (amount as unknown as Balance).toBigInt(),
    });

    // Get attestationChainData
    const chainKeyNumber = BigInt(chainKey.toString());
    const chainData = await getChainData(chainKeyNumber);
    if (chainData) {
        chainData.minBondRequirement = (amount as unknown as Balance).toBigInt();
        await chainData?.save();
    }

    await minBondRequirementUpdated.save();
}

export async function handleEventCheckpointsCleared(event: SubstrateEvent): Promise<void> {
    logger.info(`New CheckpointsCleared event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const checkpointsCleared = CheckpointsCleared.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
    });

    await checkpointsCleared.save();
}

export async function handleEventClearedStorageForRemovedChain(event: SubstrateEvent): Promise<void> {
    logger.info(`New ClearedStorageForRemovedChain event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const clearedStorageForRemovedChain = ClearedStorageForRemovedChain.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        whoId: from.toString(),
    });

    await clearedStorageForRemovedChain.save();
}

export async function handleEventAttestationIntervalChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`New AttestationIntervalChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, chainAttestationIntervalType],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const attestationIntervalChanged = AttestationIntervalChanged.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        interval: BigInt(chainAttestationIntervalType.toString()),
    });

    logger.info(
        `Going to update chainKey ${chainKeyNumber} with attestationInterval ${chainAttestationIntervalType.toString()}`,
    );
    const data = await getChainData(chainKeyNumber);
    if (data) {
        logger.info(`AttestationIntervalChanged event found for chainKey ${chainKeyNumber}`);
        data.attestationInterval = BigInt(chainAttestationIntervalType.toString());
        await data.save();
    }

    await attestationIntervalChanged.save();
}

export async function handleEventPendingAttestationIntervalSet(event: SubstrateEvent): Promise<void> {
    logger.info(`New PendingAttestationIntervalSet event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, chainAttestationIntervalType],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const pendingAttestationIntervalSet = PendingAttestationIntervalSet.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        interval: BigInt(chainAttestationIntervalType.toString()),
        whoId: from.toString(),
    });

    await pendingAttestationIntervalSet.save();
}

export async function handleCheckpointIntervalChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`New CheckpointIntervalChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, checkpointInterval],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const checkpointIntervalChanged = CheckpointIntervalChanged.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        interval: parseInt(checkpointInterval.toString(), 10),
    });

    const data = await getChainData(chainKeyNumber);
    if (data) {
        data.checkpointInterval = parseInt(checkpointInterval.toString(), 10);
        await data.save();
    }

    await checkpointIntervalChanged.save();
}

export async function handleMaxAttestorsChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`New MaxAttestorsChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, maxSetSize],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const maxAttestorsChanged = MaxAttestorsChanged.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        whoId: from.toString(),
        chainKey: BigInt(chainKey.toString()),
        eventNewMaxSize: parseInt(maxSetSize.toString(), 10),
    });

    // Update attestationChainData
    const chainKeyNumber = BigInt(chainKey.toString());
    const data = await getChainData(chainKeyNumber);
    if (data) {
        data.maxSetSize = parseInt(maxSetSize.toString(), 10);
        await data.save();
    }

    await maxAttestorsChanged.save();
}

export async function handleEventVoteAcceptanceWindowChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`New VoteAcceptanceWindowChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, u64],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const voteAcceptanceWindowChanged = VoteAcceptanceWindowChanged.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        voteAcceptanceWindow: BigInt(u64.toString()),
    });

    logger.info(`Going to update chainKey ${chainKeyNumber} with voteAcceptanceWindow ${u64.toString()}`);
    const data = await getChainData(chainKeyNumber);
    if (data) {
        logger.info(`VoteAcceptanceWindowChanged event found for chainKey ${chainKeyNumber}`);
        data.voteAcceptanceWindow = BigInt(u64.toString());
        await data.save();
    }

    await voteAcceptanceWindowChanged.save();
}

export async function handleAttestorElectionPolicyChanged(event: SubstrateEvent): Promise<void> {
    logger.info(`AttestorElectionPolicyChanged event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, newPolicy],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const electionPolicy = newPolicy.toString();

    const attestorElectionPolicyChanged = ChangedElectionPolicy.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        electionPolicy,
    });

    logger.info(`Going to update chainKey ${chainKeyNumber} with electionPolicy ${electionPolicy}`);
    const data = await getChainData(chainKeyNumber);
    if (data) {
        logger.info(`AttestorElectionPolicyChanged event found for chainKey ${chainKeyNumber}`);
        data.electionPolicy = electionPolicy;
        await data.save();
    }

    await attestorElectionPolicyChanged.save();
}

export async function handleAuthorizedAttestorAdded(event: SubstrateEvent): Promise<void> {
    logger.info(`AuthorizedAttestorAdded event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const authorizedAttestorAdded = AuthorizedAttestorAdded.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        attestorId: attestor.toString(),
    });

    const authorizedAttestor = AuthorizedAttestors.create({
        id: `${chainKeyNumber.toString()}-${attestor.toString()}`,
        chainKey: chainKeyNumber,
        attestorId: attestor.toString(),
    });

    logger.info(`Going to create authorized attestor ${attestor.toString()} for chainKey ${chainKeyNumber}`);

    const promiseList = [authorizedAttestorAdded.save(), authorizedAttestor.save()];

    await Promise.all(promiseList);
}

export async function handleAuthorizedAttestorRemoved(event: SubstrateEvent): Promise<void> {
    logger.info(`AuthorizedAttestorRemoved event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, attestor],
        },
    } = event;

    const blockNumber = event.block.block.header.number.toBigInt();

    const chainKeyNumber = BigInt(chainKey.toString());

    const authorizedAttestorRemoved = AuthorizedAttestorRemoved.create({
        id: `${blockNumber}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        attestorId: attestor.toString(),
    });

    // Remove the authorized attestor entry
    const removeAuthorizedAttestor = AuthorizedAttestors.remove(`${chainKeyNumber.toString()}-${attestor.toString()}`);

    logger.info(`Going to remove authorized attestor ${attestor.toString()} for chainKey ${chainKeyNumber}`);

    const promiseList = [authorizedAttestorRemoved.save(), removeAuthorizedAttestor];

    await Promise.all(promiseList);
}
