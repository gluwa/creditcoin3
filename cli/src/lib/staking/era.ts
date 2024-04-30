import { ApiPromise } from '..';

export async function checkEraIsInHistory(era: number, api: ApiPromise): Promise<boolean> {
    const currentEra = (await api.query.staking.currentEra()).value.toNumber();
    const historyDepth = api.consts.staking.historyDepth.toNumber();
    return eraIsInHistory(era, historyDepth, currentEra);
}

export function eraIsInHistory(eraToCheck: number, historyDepth: number, currentEra: number): boolean {
    if (eraToCheck < 0) {
        return false;
    }
    // The oldest era in history is currentEra - historyDepth
    // https://polkadot.js.org/docs/kusama/constants/#historydepth-u32
    const oldestEraInHistory = currentEra - historyDepth;
    if (eraToCheck < oldestEraInHistory || eraToCheck >= currentEra) {
        return false;
    } else {
        return true;
    }
}
