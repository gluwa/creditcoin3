import { WebSocketProvider, ethers } from 'ethers';

// BLOCKHASH(n) must return a hash only for the 256 blocks before the current one, and zero for
// anything else. It used to return the genesis hash for n = 0 at any height, and a hash for the
// current block.
describe('EVM BLOCKHASH window', (): void => {
    let provider: WebSocketProvider;

    // Runtime: return (BLOCKHASH(calldata[0..32]), NUMBER) as two 32-byte words.
    //   6000 PUSH1 0 · 35 CALLDATALOAD · 40 BLOCKHASH · 6000 PUSH1 0 · 52 MSTORE · 43 NUMBER ·
    //   6020 PUSH1 32 · 52 MSTORE · 6040 PUSH1 64 · 6000 PUSH1 0 · f3 RETURN
    const PROBE = '0x600035406000524360205260406000f3';
    const WINDOW = 256;
    const ZERO = ethers.ZeroHash;

    // Runs the probe at block `tag` from a code override, so nothing has to be deployed.
    const blockhashAt = async (tag: string, n: number) => {
        const probe = ethers.Wallet.createRandom().address;
        const result = await provider.send('eth_call', [
            { to: probe, data: ethers.zeroPadValue(ethers.toBeHex(n), 32) },
            tag,
            { [probe]: { code: PROBE } },
        ]);
        return { hash: ethers.dataSlice(result, 0, 32), current: Number(ethers.dataSlice(result, 32, 64)) };
    };

    const ethBlockHash = async (n: number): Promise<string> =>
        (await provider.send('eth_getBlockByNumber', [ethers.toQuantity(n), false])).hash;

    let blockTag: string;
    // The block number BLOCKHASH sees at `blockTag`.
    let current: number;

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        // Pin one block so every call and lookup below sees the same chain.
        blockTag = ethers.toQuantity(await provider.getBlockNumber());
        current = (await blockhashAt(blockTag, 0)).current;
        expect(current).toBeGreaterThan(1);
    });

    afterAll(async () => {
        await provider.destroy();
    });

    test('the previous block is in the window', async () => {
        expect((await blockhashAt(blockTag, current - 1)).hash).toEqual(await ethBlockHash(current - 1));
    });

    test('the current and future blocks are not', async () => {
        expect((await blockhashAt(blockTag, current)).hash).toEqual(ZERO);
        expect((await blockhashAt(blockTag, current + 1)).hash).toEqual(ZERO);
    });

    test('genesis is only reachable within the window', async () => {
        const genesis = await blockhashAt(blockTag, 0);
        if (current > WINDOW) {
            expect(genesis.hash).toEqual(ZERO);
        } else {
            // The CI chain is usually this young; an older one (e.g. devnet) takes the branch above.
            console.log(`chain at ${current} is within ${WINDOW} blocks of genesis`);
            expect(genesis.hash).toEqual(await ethBlockHash(0));
        }
    });

    test('the window is exactly 256 blocks deep', async () => {
        if (current < WINDOW + 1) {
            console.log(`chain at ${current} is too young to reach past the window`);
            return;
        }
        expect((await blockhashAt(blockTag, current - WINDOW)).hash).toEqual(await ethBlockHash(current - WINDOW));
        expect((await blockhashAt(blockTag, current - WINDOW - 1)).hash).toEqual(ZERO);
    });
});
