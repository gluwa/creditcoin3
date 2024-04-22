import { ApiPromise } from '@polkadot/api';

interface ChainStatus {
    name: string;
    bestNumber: number;
    bestFinalizedNumber: number;
    eraInfo: EraInfo;
}

export async function getChainStatus(api: ApiPromise): Promise<ChainStatus> {
    const [bestBlock, bestFinalized, eraInfo] = await Promise.all([
        api.rpc.chain.getBlock(),
        api.rpc.chain.getBlock(await api.rpc.chain.getFinalizedHead()),
        getEraInfo(api),
    ]);

    return {
        name: api.runtimeVersion.specName.toString(),
        bestNumber: bestBlock.block.header.number.toNumber(),
        bestFinalizedNumber: bestFinalized.block.header.number.toNumber(),
        eraInfo,
    };
}

interface EraInfo {
    /// The active era is the era being currently rewarded. Validator set of this era must be
    /// equal to [`SessionInterface::validators`].
    activeEra: number;
    /// This is the latest planned era, depending on how the Session pallet queues the validator
    /// set, it might be active or not.
    currentEra: number;
    /// ^^^ NOTE: the name currentEra is misleading b/c the current one is the active one!
    currentSession: number;
    sessionsPerEra: number;
}

async function getEraInfo(api: ApiPromise): Promise<EraInfo> {
    const [session, currentEra] = await Promise.all([api.derive.session.info(), api.query.staking.currentEra()]);

    return {
        activeEra: session.activeEra.toNumber(),
        currentEra: Number(currentEra),
        currentSession: (session.currentIndex.toNumber() % session.sessionsPerEra.toNumber()) + 1,
        sessionsPerEra: session.sessionsPerEra.toNumber(),
    };
}
