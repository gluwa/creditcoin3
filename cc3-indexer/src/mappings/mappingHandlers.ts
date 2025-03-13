import assert from 'assert';
import { SubstrateEvent } from '@subql/types';
import {
    AttestorsElected,
    AttestorRegistered,
    AttestorUnregistered,
    InvulnerableRegistered,
    InvulnerableUnregistered,
    TargetSampleSizeChanged,
    Bonded,
    Unbonded,
    Withdrawn,
    RewardClaimed,
    RewardPaid,
    AttestorActivated,
    AttestorChilled,
    MinBondRequirementUpdated,
    ChainRewardUpdated,
    CheckpointsCleared,
    ClearedStorageForRemovedChain,
    AttestationIntervalChanged,
    PendingAttestationIntervalSet,
    Attestors,
    Checkpoints,
    Attestations,
    MapAttestationAttestor,
    CheckpointIntervalChanged,
} from '../types';
import { Balance } from '@polkadot/types/interfaces';
import { getChainData, updateAllChainsMinBondRequirement } from './initStore';

export async function handleEventAttestorsElected(event: SubstrateEvent): Promise<void> {
    logger.info(`New Attestors Elected event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [epoch, chainKey, attestors],
        },
    } = event;

    const chainKeyStr = chainKey.toString();
    const epochStr = epoch.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);
    const epochNumber = parseInt(epochStr, 10);

    const saveEntityList = [];

    if (Array.isArray(attestors)) {
        for (let index = 0; index < attestors.length; index++) {
            const account = attestors[index];
            console.log('Processing account:', account);

            const accountStr = account.toString();

            const attestorsElected = AttestorsElected.create({
                id: `${event.block.block.header.number.toNumber()}-${event.idx}-${index}`,
                epoch: BigInt(epochNumber),
                chainKey: chainKeyNumber,
                attestorId: accountStr,
            });

            saveEntityList.push(attestorsElected.save());

            const id = `${event.block.block.header.number.toNumber()}-${event.idx}-${index}`;
            const attestorEntity = await checkAndGetAttestor(id, accountStr, chainKeyNumber);
            attestorEntity.lastUpdateBlockNumber = event.block.block.header.number.toNumber();
            attestorEntity.status = 3;

            saveEntityList.push(attestorEntity.save());
        }
    } else {
        logger.error(`Attestors is not a valid at: ${event.block.block.header.number.toString()}`);
    }

    try {
        await Promise.all(saveEntityList);
        logger.info(
            `All attestors have been dynamically added and saved at block: ${event.block.block.header.number.toString()}`,
        );
    } catch (error) {
        logger.error(
            `An error occurred while saving attestorsElected at block: ${event.block.block.header.number.toString()}`,
        );
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const attestorRegistered = AttestorRegistered.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        stashId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    const id = `${event.block.block.header.number.toNumber()}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.stashId = from.toString();
    attestorEntity.status = 1;

    logger.info(`New AttestorEntity event created at block ${event.block.block.header.number.toString()}`);

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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const attestorUnregistered = AttestorUnregistered.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
    });

    const id = `${event.block.block.header.number.toNumber()}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 2;

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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const invulnerableRegistered = InvulnerableRegistered.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const invulnerableUnregistered = InvulnerableUnregistered.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    /* eslint-disable @typescript-eslint/naming-convention */
    const checkpointReached = Checkpoints.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        whoId: from.toString(),
        atBlockNumber: blockNumber,
        chainKey: chainKeyNumber,
        blockNumber: checkpoint.blockNumber,
        digest: checkpoint.digest,
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

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);
    const newTargetSampleSizeNumber = parseInt(newTargetSampleSize.toString(), 10);

    /* eslint-disable @typescript-eslint/naming-convention */
    const targetSampleSizeChanged = TargetSampleSizeChanged.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        whoId: from.toString(),
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

function isEmpty(value: any): boolean {
    if (value == null) return true; // Checks for null or undefined
    if (typeof value === 'string' || Array.isArray(value)) return value.length === 0;
    if (typeof value === 'object') return Object.keys(value).length === 0;
    return false;
}

async function checkAndGetAttestor(id: string, attestorId: string, chainKey: number): Promise<Attestors> {
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
            lastUpdateBlockNumber: 0,
            status: 0,
            stashId: '',
            blsPublicKey: '',
        });
    }
    return attestor[0];
}

interface AttestationCheckpointData {
    blockNumber: number;
    digest: string;
}

function parseAttestationCheckpoint(attestationCheckpointStr: string): AttestationCheckpointData {
    try {
        const parsed: AttestationCheckpointData = JSON.parse(attestationCheckpointStr);

        if (typeof parsed.blockNumber !== 'number' || typeof parsed.digest !== 'string') {
            throw new Error('Invalid AttestationCheckpoint structure');
        }

        return parsed;
    } catch (error) {
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const bonded = Bonded.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as Balance).toBigInt(),
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const unbonded = Unbonded.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as Balance).toBigInt(),
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const withdrawn = Withdrawn.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as Balance).toBigInt(),
    });

    await withdrawn.save();
}

export async function handleEventRewardClaimed(event: SubstrateEvent): Promise<void> {
    logger.info(`New RewardClaimed event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [stash, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber: number = event.block.block.header.number.toNumber();

    const rewardClaimed = RewardClaimed.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        stashId: stash.toString(),
        amount: (amount as Balance).toBigInt(),
    });

    await rewardClaimed.save();
}

export async function handleEventRewardPaid(event: SubstrateEvent): Promise<void> {
    logger.info(`New RewardPaid event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, stash, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const rewardPaid = RewardPaid.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        chainKey: chainKeyNumber,
        stashId: stash.toString(),
        amount: (amount as Balance).toBigInt(),
    });

    await rewardPaid.save();
}

export async function handleEventBlockAttested(event: SubstrateEvent): Promise<void> {
    logger.info(`Block Attested event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, signedAttestation, digest],
        },
    } = event;

    logger.info(`Block Attested ${signedAttestation.toString()}`);

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const signedAttestationParsed = parseSignedAttestation(signedAttestation.toString());
    logger.info(`Block Attested signature is ${signedAttestationParsed.signature}`);

    const blockNumber: number = event.block.block.header.number.toNumber();

    const attestationId = `${event.block.block.header.number.toNumber()}-${event.idx}`;

    // /* eslint-disable @typescript-eslint/naming-convention */
    const blockAttested = Attestations.create({
        id: attestationId,
        chainKey: signedAttestationParsed.attestation.chainKey,
        headerNumber: signedAttestationParsed.attestation.headerNumber,
        headerHash: signedAttestationParsed.attestation.headerHash,
        root: signedAttestationParsed.attestation.root,
        prevDigest: signedAttestationParsed.attestation.prevDigest ?? '',
        signature: signedAttestationParsed.signature,
        digest: digest.toString(),
    });
    // /* eslint-enable */

    const saveEntityList = [blockAttested.save()];
    for (let index = 0; index < signedAttestationParsed.attestors.length; index++) {
        const id = `${event.block.block.header.number.toNumber()}-${event.idx}-${index}`;
        const attestor = signedAttestationParsed.attestors[index];
        const attestorEntity = await checkAndGetAttestor(id, attestor, chainKeyNumber);
        const blockAttestor = MapAttestationAttestor.create({
            id,
            attestorId: attestorEntity.id,
            attestationId,
        });
        saveEntityList.push(blockAttestor.save());
        logger.info(`Saved map for attestor ${attestor} and attestation ${attestationId} at block ${blockNumber}`);
    }

    logger.info(`Block Attested event stored at block ${event.block.block.header.number.toString()}`);

    const chainData = await getChainData(chainKeyNumber);
    if (chainData) {
        chainData.lastAttestedHeaderNumber = signedAttestationParsed.attestation.headerNumber;
        chainData.lastAttestedDigest = digest.toString();
        await chainData?.save();
    }

    await Promise.all(saveEntityList);
}

interface Attestation {
    chainKey: number;
    headerNumber: number;
    headerHash: string;
    root: string;
    prevDigest: string;
}

interface SignedAttestation {
    attestation: Attestation;
    signature: string;
    attestors: string[];
}

function parseSignedAttestation(attestationCheckpointStr: string): SignedAttestation {
    try {
        const parsed: SignedAttestation = JSON.parse(attestationCheckpointStr);

        if (typeof typeof parsed.signature !== 'string') {
            throw new Error('Invalid SignedAttestation structure');
        }

        return parsed;
    } catch (error) {
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    let blsPublicKeyStr = '';
    if (blsPublicKey) {
        logger.info(
            `blsPublicKey at block ${event.block.block.header.number.toString()} is ${blsPublicKey.toString()}`,
        );
        blsPublicKeyStr = blsPublicKey.toString();
    }

    const attestorActivated = AttestorActivated.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
        date: event.block.timestamp,
        blsPublicKey: blsPublicKeyStr,
    });

    const id = `${event.block.block.header.number.toNumber()}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 4;
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyStr = chainKey.toString();
    const chainKeyNumber = parseInt(chainKeyStr, 10);

    const attestorChilled = AttestorChilled.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        whoId: from.toString(),
        blockNumber,
        attestorId: attestor.toString(),
        chainKey: chainKeyNumber,
        date: event.block.timestamp,
    });

    const id = `${event.block.block.header.number.toNumber()}-${event.idx}`;
    const attestorEntity = await checkAndGetAttestor(id, attestor.toString(), chainKeyNumber);
    attestorEntity.lastUpdateBlockNumber = blockNumber;
    attestorEntity.status = 5;

    await Promise.all([attestorChilled.save(), attestorEntity.save()]);
}

export async function handleEventMinBondRequirementUpdated(event: SubstrateEvent): Promise<void> {
    logger.info(`New MinBondRequirementUpdated event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber: number = event.block.block.header.number.toNumber();

    const minBondRequirementUpdated = MinBondRequirementUpdated.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        amount: (amount as Balance).toBigInt(),
    });

    await updateAllChainsMinBondRequirement((amount as Balance).toBigInt());

    await minBondRequirementUpdated.save();
}

export async function handleEventChainRewardUpdated(event: SubstrateEvent): Promise<void> {
    logger.info(`New ChainRewardUpdated event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey, amount],
        },
    } = event;

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const chainRewardUpdated = ChainRewardUpdated.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        whoId: from.toString(),
        amount: (amount as Balance).toBigInt(),
        chainKey: chainKeyNumber,
    });

    const data = await getChainData(chainKeyNumber);
    if (data) {
        data.chainReward = (amount as Balance).toBigInt();
        await data.save();
    }

    await chainRewardUpdated.save();
}

export async function handleEventCheckpointsCleared(event: SubstrateEvent): Promise<void> {
    logger.info(`New CheckpointsCleared event found at block ${event.block.block.header.number.toString()}`);

    const {
        event: {
            data: [chainKey],
        },
    } = event;

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const checkpointsCleared = CheckpointsCleared.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const clearedStorageForRemovedChain = ClearedStorageForRemovedChain.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const attestationIntervalChanged = AttestationIntervalChanged.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        interval: parseInt(chainAttestationIntervalType.toString(), 10),
    });

    const data = await getChainData(chainKeyNumber);
    if (data) {
        data.attestationInterval = parseInt(chainAttestationIntervalType.toString(), 10);
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const from = event.extrinsic?.extrinsic.signer;
    assert(from, 'Signer is missing');

    const pendingAttestationIntervalSet = PendingAttestationIntervalSet.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
        blockNumber,
        date: event.block.timestamp,
        chainKey: chainKeyNumber,
        interval: parseInt(chainAttestationIntervalType.toString(), 10),
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

    const blockNumber: number = event.block.block.header.number.toNumber();

    const chainKeyNumber = parseInt(chainKey.toString(), 10);

    const checkpointIntervalChanged = CheckpointIntervalChanged.create({
        id: `${event.block.block.header.number.toNumber()}-${event.idx}`,
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
