import { SubstrateBlock } from '@subql/types';
import { AttestationChainData, SupportedChain } from '../types';

export async function initiateStoreAndDatabase(block: SubstrateBlock): Promise<void> {
    logger.info(`Initiating store and database at block #${block.block.header.number.toString()}`);

    const chain1 = AttestationChainData.create({
        id: 'chain_1',
        chainKey: BigInt(1),
        attestationInterval: BigInt(10),
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: BigInt(0),
        lastCheckpointHeaderNumber: BigInt(0),
        maxSetSize: 100,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100000000000000000000),
        voteAcceptanceWindow: BigInt(3),
    });
    const supportedChain1 = SupportedChain.create({
        id: 'chain_1',
        chainKey: BigInt(1),
        chainName: 'Ethereum',
        chainId: BigInt(1),
        at: block.block.header.number.toBigInt(),
    });
    await Promise.all([chain1.save(), supportedChain1.save()]);

    const chain2 = AttestationChainData.create({
        id: 'chain_2',
        chainKey: BigInt(2),
        attestationInterval: BigInt(10),
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: BigInt(0),
        lastCheckpointHeaderNumber: BigInt(0),
        maxSetSize: 100,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100000000000000000000),
        voteAcceptanceWindow: BigInt(3),
    });
    const supportedChain2 = SupportedChain.create({
        id: 'chain_2',
        chainKey: BigInt(2),
        chainName: 'Anvil1',
        chainId: BigInt(31337),
        at: block.block.header.number.toBigInt(),
    });
    await Promise.all([chain2.save(), supportedChain2.save()]);

    const chain3 = AttestationChainData.create({
        id: 'chain_3',
        chainKey: BigInt(3),
        attestationInterval: BigInt(10),
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: BigInt(0),
        lastCheckpointHeaderNumber: BigInt(0),
        maxSetSize: 100,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100000000000000000000),
        voteAcceptanceWindow: BigInt(3),
    });
    const supportedChain3 = SupportedChain.create({
        id: 'chain_3',
        chainKey: BigInt(3),
        chainName: 'Sepolia ethereum',
        chainId: BigInt(11155111),
        at: block.block.header.number.toBigInt(),
    });
    await Promise.all([chain3.save(), supportedChain3.save()]);

    const chain4 = AttestationChainData.create({
        id: 'chain_4',
        chainKey: BigInt(4),
        attestationInterval: BigInt(10),
        checkpointInterval: 10,
        chainReward: BigInt(1000),
        lastAttestedDigest: '',
        lastAttestedHeaderNumber: BigInt(0),
        lastCheckpointHeaderNumber: BigInt(0),
        maxSetSize: 100,
        targetSampleSize: 3,
        minBondRequirement: BigInt(100000000000000000000),
        voteAcceptanceWindow: BigInt(3),
    });
    const supportedChain4 = SupportedChain.create({
        id: 'chain_4',
        chainKey: BigInt(4),
        chainName: 'Anvil2',
        chainId: BigInt(31338),
        at: block.block.header.number.toBigInt(),
    });
    await Promise.all([chain4.save(), supportedChain4.save()]);
}

/* eslint-disable @typescript-eslint/no-redundant-type-constituents */
export async function getChainData(chainKey: bigint): Promise<AttestationChainData | null> {
    const a = await AttestationChainData.getByChainKey(chainKey, { limit: 1 });
    return a[0];
}
