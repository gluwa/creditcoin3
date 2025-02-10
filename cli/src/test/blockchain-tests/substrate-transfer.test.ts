import { WebSocketProvider, ethers, parseEther } from 'ethers';
// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABI = require('./artifacts/SubstrateTransfer.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../lib';
import { fundFromSudo } from '../integration-tests/helpers';

describe('Precompile: transfer_substrate()', (): void => {
    let contract: any;
    let destination: any;
    let destinationBalanceBefore: bigint;
    let provider: any;
    let alith: any;
    let alithBalanceBefore: bigint;
    let api: ApiPromise;
    let gasPrice: bigint;
    let gasLimit: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // precompile contract deployed at 4049 to hex, see runtime/src/precompiles.rs for more
        const precompileContractAddress = '0x0000000000000000000000000000000000000fd1';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);
        // will only work when connected to a chain locally and //Alice is root
        // either during local development or during runtime-upgrade against a fork
        // note: Alith starts with 2mil CTC during local development
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        // note: balances.Transfer is happy to accept Address20 directly too
        expect(result.status).toBe(0);
        alithBalanceBefore = await provider.getBalance(alith.address);

        contract = new ethers.Contract(precompileContractAddress, contractABI, alith);
        const target = new Keyring();
        destination = target.addFromMnemonic(mnemonicGenerate());

        destinationBalanceBefore = (await api.derive.balances.all(destination.address)).availableBalance.toBigInt();

        gasLimit = 10000000;
        // note: larger timeout b/c this also executes against Testnet forks where block time is 15s
    }, 90_000);

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
            gasLimit,
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
        const totalIssuance = (await api.query.balances.totalIssuance()).toBigInt();
        // trying to send 1 bil more than total issuance
        const amount = totalIssuance + BigInt(1_000_000_000_000_000_000_000_000_000);

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Arithmetic\(Underflow\)/);
        // ^^^ appears to come from can_withdraw()
        // https://github.com/gluwa/polkadot-sdk/blob/master/substrate/frame/balances/src/impl_fungible.rs#L110

        // Alice may have paid gas fees regardless of the error
        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceAfter).toBeLessThanOrEqual(alithBalanceBefore);
    });

    test('should fail when sending more than available funds', async () => {
        // trying to send 1 mil more than available balance
        const amount = alithBalanceBefore + BigInt(1_000_000_000_000_000_000_000_000);

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Token\(FundsUnavailable\)/);
        // ^^^ appears to come from do_transfer_reserved()
        // https://github.com/gluwa/polkadot-sdk/blob/master/substrate/frame/balances/src/lib.rs#L1098

        // Alice may have paid gas fees regardless of the error
        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceAfter).toBeLessThanOrEqual(alithBalanceBefore);
    });
});
