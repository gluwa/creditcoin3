import { WebSocketProvider, ethers, parseEther } from 'ethers';
import contractABI = require('./artifacts/SubstrateTransfer.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';
import { newApi, ApiPromise } from '../../lib';

describe('Substrate seamless transfer precompile', (): void => {
    let contract: any;
    let amount: bigint;
    let destination: any;
    let destinationBalanceBefore: bigint;
    let destinationBalanceAfter: bigint;
    let provider: any;
    let alith: any;
    let alithBalanceBefore: bigint;
    let api: ApiPromise;
    let gasPrice: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // precompile contract deployed at 4049 to hex, see runtime/src/precompiles.rs for more
        const precompileContractAddress = '0x0000000000000000000000000000000000000fd1';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);
        alithBalanceBefore = await provider.getBalance(alith.address);

        contract = new ethers.Contract(precompileContractAddress, contractABI, alith);
        const target = new Keyring();
        destination = target.addFromMnemonic(mnemonicGenerate());

        destinationBalanceBefore = (await api.derive.balances.all(destination.address)).availableBalance.toBigInt();
    }, 25000);

    afterAll(async () => {
        await api.disconnect();
    }, 10_000);

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    }, 25000);

    test('transfer_substrate happy path', async () => {
        amount = parseEther('10.0');
        const result = await contract.transfer_substrate(destination.addressRaw, amount, {
            gasPrice,
        });
        const receipt = await result.wait();
        const txHash = result?.hash;

        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        destinationBalanceAfter = (await api.derive.balances.all(destination.address)).availableBalance.toBigInt();
        expect(txHash).toBeDefined();
        expect(receipt).toBeDefined();
        expect(alithBalanceBefore).toBe(alithBalanceAfter + amount + BigInt(receipt.cumulativeGasUsed * gasPrice));
        expect(destinationBalanceAfter).toBe(destinationBalanceBefore + BigInt(amount));
    }, 25000);

    test('transfer_substrate insufficient funds path', async () => {
        amount = parseEther('1000000000.0');
        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Arithmetic\(Underflow\)/);
    }, 25000);

    // We have different errors for insufficient funds and insufficient gas. Testcase below is for insufficient gas.
    test('transfer_substrate sufficient funds + insufficient gas path', async () => {
        amount = parseEther('1999989.9');

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Token\(FundsUnavailable\)/);
    }, 25000);
});
