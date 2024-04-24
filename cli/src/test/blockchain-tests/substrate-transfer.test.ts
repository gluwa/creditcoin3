import { WebSocketProvider, ethers, parseEther } from 'ethers';
import contractABI = require('./artifacts/SubstrateTransfer.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';
import { newApi, ApiPromise } from '../../lib';

describe('Precompile: transfer_substrate()', (): void => {
    let contract: any;
    let destination: any;
    let destinationBalanceBefore: bigint;
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
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    });

    test('should work when caller has enough funds', async () => {
        const amount = parseEther('10.0');
        const result = await contract.transfer_substrate(destination.addressRaw, amount, {
            gasPrice,
        });
        const receipt = await result.wait();
        expect(receipt).toBeDefined();

        const txHash = result?.hash;
        expect(txHash).toBeDefined();

        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceBefore).toBe(alithBalanceAfter + amount + BigInt(receipt.cumulativeGasUsed * gasPrice));

        const destinationBalanceAfter = (
            await api.derive.balances.all(destination.address)
        ).availableBalance.toBigInt();
        expect(destinationBalanceAfter).toBe(destinationBalanceBefore + BigInt(amount));
    });

    test('should fail when sending more than total issuance', async () => {
        // a local development chain starts with total issuance of 14 M CTC
        // trying to send 1 bil
        const amount = parseEther('1000000000.0');

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Arithmetic\(Underflow\)/);
        // ^^^ appears to come from can_withdraw()
        // https://github.com/gluwa/polkadot-sdk/blob/master/substrate/frame/balances/src/impl_fungible.rs#L110
    });

    test('should fail when sending more than available funds', async () => {
        // Alice starts with 1M CTC, try sending 1.9 mil
        const amount = parseEther('1999989.9');

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Token\(FundsUnavailable\)/);
        // ^^^ appears to come from do_transfer_reserved()
        // https://github.com/gluwa/polkadot-sdk/blob/master/substrate/frame/balances/src/lib.rs#L1098
    });
});
