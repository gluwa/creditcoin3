export interface EthHead {
    hash: string;
    // The Ethereum RPC header field is named "number".
    // eslint-disable-next-line id-denylist
    number: string;
    timestamp: string;
}

type ReadBlock = (method: string, params: unknown[]) => Promise<EthHead | null>;

// A subscribed head can be replaced before its hash is queried. Only discard it
// after the canonical block at that height confirms the replacement; an unreadable
// current head must still fail the test.
export const readSubscribedBlock = async (
    read: ReadBlock,
    head: EthHead,
    timeoutMs = 5_000,
): Promise<EthHead | null> => {
    const deadline = Date.now() + timeoutMs;
    do {
        const block = await read('eth_getBlockByHash', [head.hash, false]);
        if (block !== null) {
            return block;
        }
        const canonical = await read('eth_getBlockByNumber', [head.number, false]);
        if (canonical !== null && canonical.hash !== head.hash) {
            return null;
        }
        await new Promise((resolve) => setTimeout(resolve, Math.min(100, Math.max(0, deadline - Date.now()))));
    } while (Date.now() < deadline);
    throw new Error(`Subscribed block ${head.hash} at ${head.number} was not readable or replaced`);
};
