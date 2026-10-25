import { WebSocketProvider, ethers, parseEther } from 'ethers';

describe('Gas estimation with low gas', (): void => {
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;

    beforeAll(() => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);
    });

    test('eth_estimateGas succeeds for a simple transfer', async () => {
        const estimate = await provider.estimateGas({
            from: alith.address,
            to: alith.address,
            value: parseEther('1'),
        });
        expect(estimate).toBeGreaterThanOrEqual(21_000n);
    });

    test('eth_estimateGas succeeds with large input data', async () => {
        // Use a large input payload so the proof_size_base_cost is significant.
        // This is the scenario that was previously broken: the binary search in
        // eth_estimateGas would hit WeightInfo::new_from_weight_limit with a
        // gas limit too low to cover proof_size_base_cost, causing a hard error
        // instead of a soft OutOfGas that the binary search can handle.
        const largeInput = '0x' + 'aa'.repeat(10_000);

        const estimate = await provider.estimateGas({
            from: alith.address,
            to: alith.address,
            value: 0n,
            data: largeInput,
        });

        // The estimation should succeed and return a reasonable gas value
        // that accounts for the large input data.
        expect(estimate).toBeGreaterThan(21_000n);
    });

    test('submitting a transaction with too-low gas is rejected', async () => {
        // A real (transactional) call with gas far too low should be rejected
        // at the validation layer, never reaching the EVM runner.
        // This confirms the validation gate works for transactional calls.
        await expect(
            alith.sendTransaction({
                to: alith.address,
                value: parseEther('0.01'),
                gasLimit: 1, // absurdly low
            }),
        ).rejects.toThrow();
    });

    test('transaction with gas just below estimate is rejected', async () => {
        const largeInput = '0x' + 'bb'.repeat(5_000);

        const estimate = await provider.estimateGas({
            from: alith.address,
            to: alith.address,
            value: 0n,
            data: largeInput,
        });

        // Try sending with gas slightly below what was estimated.
        // The node should reject this at validation or it should fail execution.
        const tooLowGas = estimate - estimate / 4n; // 25% below estimate

        await expect(
            alith.sendTransaction({
                to: alith.address,
                value: 0n,
                data: largeInput,
                gasLimit: tooLowGas,
            }),
        ).rejects.toThrow();
    });

    test('transaction with gas at estimate succeeds', async () => {
        const largeInput = '0x' + 'cc'.repeat(5_000);

        const estimate = await provider.estimateGas({
            from: alith.address,
            to: alith.address,
            value: 0n,
            data: largeInput,
        });

        // Sending with the estimated gas should succeed.
        const tx = await alith.sendTransaction({
            to: alith.address,
            value: 0n,
            data: largeInput,
            gasLimit: estimate,
        });
        const receipt = await tx.wait();
        expect(receipt).toBeDefined();
        expect(receipt!.status).toBe(1);
    });
});
