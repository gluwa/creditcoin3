import { WebSocketProvider, ethers } from 'ethers';

// Verifies the Frontier fix that lets the eth JSON-RPC block param accept a bare
// 32-byte block-hash string (as Geth / op-deployer's forking layer send it).
// Before the fix, `visit_str` parsed every `0x`-prefixed string as a hex u64 and
// overflowed on a 66-char hash with: -32602 "Invalid block number: number too
// large to fit in target type". After the fix it resolves to the Hash variant.
describe('eth block param accepts a bare block hash', (): void => {
    let provider: WebSocketProvider;
    let addr: string;
    let blockHash: string;
    let blockNumber: string;

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        addr = new ethers.Wallet(privateKey).address;

        // Grab a real recent block (hash + its equivalent number, both as the
        // node returns them — number is a hex string).
        const latest = await provider.send('eth_getBlockByNumber', ['latest', false]);
        blockHash = latest.hash;
        blockNumber = latest.number;
        expect(blockHash).toMatch(/^0x[0-9a-fA-F]{64}$/);
    }, 30_000);

    // The exact call op-deployer makes: a bare 32-byte hash as the block param.
    test('eth_getTransactionCount resolves with a bare block hash', async () => {
        const nonce = await provider.send('eth_getTransactionCount', [addr, blockHash]);
        expect(nonce).toMatch(/^0x[0-9a-fA-F]+$/);
    });

    // The bare hash must resolve to the same block as its equivalent number.
    test('nonce(by hash) equals nonce(by number)', async () => {
        const nonceByHash = await provider.send('eth_getTransactionCount', [addr, blockHash]);
        const nonceByNumber = await provider.send('eth_getTransactionCount', [addr, blockNumber]);
        expect(nonceByHash).toBe(nonceByNumber);
    });

    // Sibling state methods share the same block-param resolution path.
    test.each([
        ['eth_getCode', () => [addr, blockHash]],
        ['eth_getBalance', () => [addr, blockHash]],
        ['eth_getStorageAt', () => [addr, '0x0', blockHash]],
        ['eth_call', () => [{ to: addr }, blockHash]],
    ])('%s accepts a bare block hash', async (method, params) => {
        await expect(provider.send(method, (params as () => any[])())).resolves.toBeDefined();
    });

    // Regression: the existing block-param forms must keep working.
    test('eth_getTransactionCount still accepts the existing block-param forms', async () => {
        const forms: any[] = ['latest', blockNumber, { blockHash }, { blockNumber }];
        for (const blockParam of forms) {
            await expect(provider.send('eth_getTransactionCount', [addr, blockParam])).resolves.toMatch(
                /^0x[0-9a-fA-F]+$/,
            );
        }
    });
});
