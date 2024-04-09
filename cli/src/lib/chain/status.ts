import { ApiPromise } from '@polkadot/api';

interface ChainStatus {
    name: string;
    bestNumber: number;
    bestFinalizedNumber: number;
    eraInfo: EraInfo;
}

export async function getChainStatus(api: ApiPromise): Promise<ChainStatus> {
    const bestNumber = await api.derive.chain.bestNumber();
    const bestFinalizedNumber = await api.derive.chain.bestNumberFinalized();
    const eraInfo = await getEraInfo(api);
    return {
        name: api.runtimeVersion.specName.toString(),
        bestNumber: bestNumber.toNumber(),
        bestFinalizedNumber: bestFinalizedNumber.toNumber(),
        eraInfo,
    };
}

interface EraInfo {
    era: number;
    currentSession: number;
    sessionsPerEra: number;
}

async function getEraInfo(api: ApiPromise): Promise<EraInfo> {
    const session = await api.derive.session.info();
    return {
        era: session.activeEra.toNumber(),
        currentSession: (session.currentIndex.toNumber() % session.sessionsPerEra.toNumber()) + 1,
        sessionsPerEra: session.sessionsPerEra.toNumber(),
    };
}
