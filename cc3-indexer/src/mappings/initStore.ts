import { SubstrateBlock } from '@subql/types';
import { AttestationChainData, SupportedChain } from '../types';
import type { ApiPromise } from '@polkadot/api';

// SubQuery injects a height-scoped ApiPromise into the sandbox
declare const api: ApiPromise;

const BI = (v: number | string) => BigInt(v);

export async function initiateStoreAndDatabase(block: SubstrateBlock): Promise<void> {
    logger.info(`--- Initiating store and database at block #${block.block.header.number.toString()}`);

    // Read all supported chains at the current indexed height
    const rawEntries = await (api.query as any).supportedChains.supportedChains.entries();

    const entries: [bigint, { chainId: bigint; chainName: string; chainEncoding: string; maturityStrategy: string }][] =
        rawEntries.map(([storageKey, value]: any) => {
            const chainKey = BI(storageKey.args[0].toString());
            const j = value?.toJSON?.() ?? {};
            const chainId = value?.chainId?.toBigInt?.() ?? (j.chainId != null ? BI(j.chainId as number) : BI(0));
            const chainNameHex =
                value?.chainName?.toHex?.() ??
                value?.chainName?.toString?.() ??
                (typeof j.chainName === 'string' ? j.chainName : '0x');
            const chainEncoding =
                value?.chainEncoding?.toString?.() ?? (typeof j.chainEncoding === 'string' ? j.chainEncoding : 'V1');
            const maturityStrategy =
                value?.maturityStrategy?.toString?.() ??
                (typeof j.maturityStrategy === 'string' ? j.maturityStrategy : '');
            if (maturityStrategy == null) {
                throw new Error(`maturityStrategy missing for chainKey= ${chainKey.toString()}`);
            }
            return [chainKey, { chainId, chainName: chainNameHex, chainEncoding, maturityStrategy }];
        });

    for (const [chainKey, { chainId, chainName, chainEncoding, maturityStrategy }] of entries) {
        const id = `chain_${chainKey.toString()}`;
        logger.info(`Processing chain ${id} with key ${chainKey.toString()}`);
        logger.info(`Chain ID: ${chainId.toString()}`);
        logger.info(`Chain Name (Hex): ${chainName}`);
        const name = Buffer.from(chainName.toString().slice(2), 'hex').toString('utf8');

        const att = (api.query as any).attestation;

        // attestation pallet lookups (your list)
        const attestationInterval = (await att.chainAttestationInterval(chainKey)).toBigInt(); // u64
        const checkpointInterval = (await att.attestationCheckpointInterval(chainKey)).toNumber(); // u32

        const lastDigestOpt = await att.lastDigest(chainKey); // Option<H256>
        const lastAttestedDigest = lastDigestOpt && lastDigestOpt.isSome ? lastDigestOpt.unwrap().toHex() : '';

        const maxSetSize = (await att.maxAttestors(chainKey)).toNumber(); // u32
        const targetSampleSize = (await att.targetSampleSize(chainKey)).toNumber(); // u32

        const electionPolicy = await att.chainElectionPolicy(chainKey); // AttestorElectionPolicy
        const electionPolicyValue = electionPolicy.toString();

        // Need this for devnet as this storage item was upgraded during it's lifetime
        let minBondRequirement = BigInt(100000000000000000000); // u128, default to 100000000000000000000 if not set
        try {
            minBondRequirement = (await att.minBondRequirement(chainKey)).toBigInt(); // u128
        } catch {
            logger.warn(`minBondRequirement not found for chainKey ${chainKey}, defaulting to 100000000000000000000`);
        }

        let voteAcceptanceWindow = BI(3); // u32, default to 3 if not set
        try {
            voteAcceptanceWindow = (await att.voteAcceptanceWindow(chainKey)).toBigInt(); //
        } catch {
            logger.warn(`voteAcceptanceWindow not found for chainKey ${chainKey}, defaulting to 3`);
        }

        const supported = SupportedChain.create({
            id,
            chainKey,
            chainName: name,
            chainId,
            chainEncoding,
            maturityStrategy,
            at: block.block.header.number.toBigInt(),
        });

        const acd = AttestationChainData.create({
            id,
            chainKey,
            chainReward: BI(0),
            attestationInterval,
            checkpointInterval,
            lastAttestedDigest,
            lastAttestedHeaderNumber: BI(0), // fill in later if you want to derive these
            lastCheckpointHeaderNumber: BI(0),
            maxSetSize,
            targetSampleSize,
            minBondRequirement,
            voteAcceptanceWindow,
            electionPolicy: electionPolicyValue,
        });

        await Promise.all([supported.save(), acd.save()]);
        logger.info(`Saved ${id}(${name})`);
    }
}

export async function getChainData(chainKey: bigint) {
    const a = await AttestationChainData.getByChainKey(chainKey, { limit: 1 });
    return a[0];
}
