import { WebSocketProvider, ethers, parseEther } from 'ethers';
import contractABI = require("./artifacts/SubstrateTransfer.json");

describe('Substrate seamless transfer precompile', (): void => {
    let provider: WebSocketProvider;
    let precompileContractAddress: string;
    let txHash: string;
    let receipt: string;
    let bobBalance: bigint;
    let bobBalanceAfter: bigint;

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        const alith = new ethers.Wallet(privateKey).connect(provider);

        // precompile contract deployed at 4049 to hex
        precompileContractAddress = "0x0000000000000000000000000000000000000fd1";

        const contract = new ethers.Contract(
            precompileContractAddress,
            contractABI,
            alith
        );

        const balance = await provider.getBalance(alith.address);

        const bobKeyring = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        bobBalance = await provider.getBalance(bobKeyring?.address);

        const amount = parseEther("10.0");
        const gasPrice = (await provider.getFeeData()).gasPrice;

        const result = await contract.transfer_substrate(bobKeyring?.address, amount, {
            gasPrice,
        });
        receipt = await result.wait();
        txHash = result?.hash;

        bobBalanceAfter = await provider.getBalance(bobKeyring?.address);
    }, 25000);

    test('substrate_transfer', () => {
        expect(txHash).toBeDefined();
        expect(receipt).toBeDefined();
        expect(bobBalanceAfter).toBeGreaterThan(bobBalance);
    });
});