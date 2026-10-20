import { WebSocketProvider, ethers } from 'ethers';

// `eth_estimateGas` answers a flat 21,000 for a plain transfer without simulating it. That shortcut
// must only apply when nothing else costs gas: no access list, no authorization list and no
// precompile target. It used to answer 21,000 whenever there was no calldata and no code at `to`.
describe('eth_estimateGas transfer shortcut', (): void => {
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;
    let chainId: number;

    const TRANSFER = 21_000n;
    // EIP-2930 intrinsic costs.
    const ACCESS_LIST_ADDRESS = 2_400n;
    const ACCESS_LIST_STORAGE_KEY = 1_900n;
    // EIP-7702 intrinsic cost per authorization.
    const AUTHORIZATION = 25_000n;
    // The ECRecover precompile.
    const ECRECOVER = '0x0000000000000000000000000000000000000001';

    const estimate = async (request: Record<string, unknown>): Promise<bigint> =>
        BigInt(await provider.send('eth_estimateGas', [{ from: alith.address, value: '0x1', ...request }]));

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);
        chainId = Number((await provider.getNetwork()).chainId);
    });

    afterAll(async () => {
        await provider.destroy();
    });

    test('a plain transfer still costs exactly 21,000', async () => {
        expect(await estimate({ to: ethers.Wallet.createRandom().address })).toEqual(TRANSFER);
    });

    test('an access list is charged', async () => {
        const gas = await estimate({
            to: ethers.Wallet.createRandom().address,
            accessList: [{ address: ethers.Wallet.createRandom().address, storageKeys: [ethers.ZeroHash] }],
        });
        expect(gas).toBeGreaterThanOrEqual(TRANSFER + ACCESS_LIST_ADDRESS + ACCESS_LIST_STORAGE_KEY);
    });

    test('an authorization list is charged', async () => {
        // Frontier's RPC takes `chainId` as a number and `yParity` as a boolean. The signature is junk:
        // the authorization is skipped when applied, but its intrinsic cost is still charged.
        const junk = `0x${'11'.repeat(32)}`;
        const gas = await estimate({
            to: ethers.Wallet.createRandom().address,
            authorizationList: [
                {
                    chainId,
                    address: ethers.Wallet.createRandom().address,
                    nonce: '0x0',
                    yParity: false,
                    r: junk,
                    s: junk,
                },
            ],
        });
        expect(gas).toBeGreaterThanOrEqual(TRANSFER + AUTHORIZATION);
    });

    test('a call to a precompile is simulated', async () => {
        // ECRecover charges 3,000 even for empty input. On chains whose precompile addresses hold no
        // code (devnet, testnet, mainnet), only the node's `is_precompile` check keeps this off the
        // shortcut. The dev chain spec CI runs on puts bytecode there, so it is simulated either way.
        expect(await estimate({ to: ECRECOVER, value: '0x0' })).toBeGreaterThan(TRANSFER);
    });
});
