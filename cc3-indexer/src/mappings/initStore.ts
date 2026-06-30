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
        // Use a single, prefix-free id scheme so this path and the `ChainRegistered` event
        // handler (`handleSupportedChainRegistered`) write the SAME row for a given chain — a
        // prefixed id here (`chain_<k>`) plus a bare id there would risk two rows per chain and
        // make `getByChainKey(... limit 1)` non-deterministic.
        const id = chainDataId(chainKey);
        logger.info(`Processing chain ${id} with key ${chainKey.toString()}`);
        logger.info(`Chain ID: ${chainId.toString()}`);
        logger.info(`Chain Name (Hex): ${chainName}`);
        const name = Buffer.from(chainName.toString().slice(2), 'hex').toString('utf8');

        const params = await fetchAttestationParams(chainKey);

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
            attestationInterval: params.attestationInterval,
            checkpointInterval: params.checkpointInterval,
            lastAttestedDigest: params.lastAttestedDigest,
            lastAttestedHeaderNumber: BI(0), // fill in later if you want to derive these
            lastCheckpointHeaderNumber: BI(0),
            maxSetSize: params.maxSetSize,
            targetSampleSize: params.targetSampleSize,
            minBondRequirement: params.minBondRequirement,
            electionPolicy: params.electionPolicy,
        });

        await Promise.all([supported.save(), acd.save()]);
        logger.info(`Saved ${id}(${name})`);
    }
}

// Canonical id for `SupportedChain` / `AttestationChainData` rows. MUST match the id used by
// `handleSupportedChainRegistered` so both creation paths upsert the same row.
export function chainDataId(chainKey: bigint): string {
    return chainKey.toString();
}

export interface AttestationParams {
    attestationInterval: bigint;
    checkpointInterval: number;
    maxSetSize: number;
    targetSampleSize: number;
    electionPolicy: string;
    minBondRequirement: bigint;
    lastAttestedDigest: string;
}

// Read the real attestation-pallet parameters for `chainKey` from chain state at the current
// indexed height. `api` is height-scoped by SubQuery, and `on_register_chain` writes these
// storage items in the same extrinsic that emits `ChainRegistered`, so they are already present
// at the registration block — no need to (and we must NOT) hardcode them.
export async function fetchAttestationParams(chainKey: bigint): Promise<AttestationParams> {
    const att = (api.query as any).attestation;

    const attestationInterval = (await att.chainAttestationInterval(chainKey)).toBigInt(); // u64
    const checkpointInterval = (await att.attestationCheckpointInterval(chainKey)).toNumber(); // u32

    const lastDigestOpt = await att.lastDigest(chainKey); // Option<H256>
    const lastAttestedDigest = lastDigestOpt && lastDigestOpt.isSome ? lastDigestOpt.unwrap().toHex() : '';

    const maxSetSize = (await att.maxAttestors(chainKey)).toNumber(); // u32
    const targetSampleSize = (await att.targetSampleSize(chainKey)).toNumber(); // u32

    const electionPolicy = (await att.chainElectionPolicy(chainKey)).toString(); // AttestorElectionPolicy

    // Need this fallback for devnet as this storage item was upgraded during its lifetime.
    let minBondRequirement = BigInt('100000000000000000000'); // u128 default
    try {
        minBondRequirement = (await att.minBondRequirement(chainKey)).toBigInt(); // u128
    } catch {
        logger.warn(`minBondRequirement not found for chainKey ${chainKey}, defaulting to 100000000000000000000`);
    }

    return {
        attestationInterval,
        checkpointInterval,
        maxSetSize,
        targetSampleSize,
        electionPolicy,
        minBondRequirement,
        lastAttestedDigest,
    };
}

export async function getChainData(chainKey: bigint) {
    const a = await AttestationChainData.getByChainKey(chainKey, { limit: 1 });
    return a[0];
}
