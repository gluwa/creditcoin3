import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { deployContract } from './helpers';
import { WebSocketProvider, ethers, parseEther } from 'ethers';

describe.only('EVM Tracing', (): void => {
    let provider: WebSocketProvider;
    let deployedContractAddress: string;
    let txHash: string;

    beforeAll(async () => {
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        const alith = new ethers.Wallet(privateKey).connect(provider);

        // deploy SendForYou Smart contract
        const contract = await deployContract('SendForYou', [], alith);
        deployedContractAddress = await contract.getAddress();

        // send funds to the contract
        const response = await alith.sendTransaction({
            to: deployedContractAddress,
            value: parseEther('50'),
        });
        await response.wait();

        // call contract method sendForMe to bob
        const bobKeyring = (global as any).CREDITCOIN_CREATE_SIGNER('borrower');

        const call = await contract
            .getFunction('sendForMe')
            .call(contract, substrateAddressToEvmAddress(bobKeyring?.address), parseEther('10'));

        await call.wait();

        txHash = call?.hash;
    }, 25000);

    test('debug_traceTransaction', async () => {
        expect(txHash).toBeDefined();

        // call rpc method `debug_traceTransaction`
        const traceTxResponse = await provider.send('debug_traceTransaction', [txHash]);
        expect(traceTxResponse?.gas).toBeDefined();
        expect(traceTxResponse?.structLogs?.length).toBeGreaterThan(0);
    });

    test('debug_traceBlockByHash', async () => {
        expect(txHash).toBeDefined();

        // get transaction block information from tx hash
        const tx = await provider.getTransaction(txHash);
        expect(tx).toBeDefined();

        // call rpc method `debug_traceBlockByHash`
        const traceBlockResponse = await provider.send('debug_traceBlockByHash', [
            tx?.blockHash,
            { tracer: 'callTracer' },
        ]);

        expect(traceBlockResponse?.[0]?.gas).toBeDefined();
        expect(traceBlockResponse?.[0]?.gasUsed).toBeDefined();
        expect(traceBlockResponse?.[0]?.type).toBe('CALL');
        expect(traceBlockResponse?.[0]?.to).toBe(deployedContractAddress.toLowerCase());
        expect(traceBlockResponse?.[0]?.calls?.length).toBe(1);
    });
});
