import { SubstrateBlock } from '@subql/types';
import { AttestationChainData } from '../types';

export async function initiateStoreAndDatabase(block: SubstrateBlock): Promise<void> {
    logger.info(`Initiating store and database at block #${block.block.header.number.toString()}`);

    const chain1 = AttestationChainData.create({
        id: 'chain_1',
        chainKey: 1,
        attestationInterval: 10,
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: 0,
        lastCheckpointHeaderNumber: 0,
        maxSetSize: 3,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100),
    });
    await chain1.save();

    const chain2 = AttestationChainData.create({
        id: 'chain_2',
        chainKey: 2,
        attestationInterval: 10,
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: 0,
        lastCheckpointHeaderNumber: 0,
        maxSetSize: 3,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100),
    });
    await chain2.save();

    const chain3 = AttestationChainData.create({
        id: 'chain_3',
        chainKey: 3,
        attestationInterval: 10,
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: 0,
        lastCheckpointHeaderNumber: 0,
        maxSetSize: 3,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100),
    });
    await chain3.save();

    const chain4 = AttestationChainData.create({
        id: 'chain_4',
        chainKey: 4,
        attestationInterval: 10,
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: 0,
        lastCheckpointHeaderNumber: 0,
        maxSetSize: 3,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100),
    });
    await chain4.save();
}

// Getter
export async function getChainData(chainKey: number): Promise<AttestationChainData | null> {
    const a = await AttestationChainData.getByChainKey(chainKey, { limit: 1 });
    return a[0];
}

// TODO: Modify pallet to allow for updating min bond requirement per chain
export async function updateAllChainsMinBondRequirement(newMinBondRequirement: bigint): Promise<void> {
    const chain1 = await getChainData(1);
    if (chain1) {
        chain1.minBondRequirement = newMinBondRequirement;
        await chain1.save();
    }

    const chain2 = await getChainData(2);
    if (chain2) {
        chain2.minBondRequirement = newMinBondRequirement;
        await chain2.save();
    }

    const chain3 = await getChainData(3);
    if (chain3) {
        chain3.minBondRequirement = newMinBondRequirement;
        await chain3.save();
    }

    const chain4 = await getChainData(4);
    if (chain4) {
        chain4.minBondRequirement = newMinBondRequirement;
        await chain4.save();
    }
}
