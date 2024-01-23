import { deployContract } from './helpers';
import { JsonRpcProvider, ethers, parseEther } from 'ethers';

describe.only('EVM Tracing', (): void => {
    let provider: JsonRpcProvider;

    beforeAll(async () => {
        provider = new JsonRpcProvider('http://127.0.0.1:9944');
    });

    test('tracing rpc methods work correctly', async () => {
        const alith = new ethers.Wallet('0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133').connect(
            provider,
        );

        // deploy SendForYou Smart contract
        const contract = await deployContract('SendForYou', [], alith);
        const contractAddress = await contract.getAddress();

        // send funds to the contract
        const response = await alith.sendTransaction({
            to: await contract.getAddress(),
            value: parseEther('50'),
        });
        await response.wait();

        // call contract method sendForMe to random address
        const call = await contract
            .getFunction('sendForMe')
            .call(contract, '0x2D8290e675564F49229a62A255C1b227aF4425D9', parseEther('10'));

        await call.wait();
        expect(call?.hash).toBeDefined();

        // test debug_traceTransaction
        const traceTxResponse = await provider.send('debug_traceTransaction', [call?.hash]);
        expect(traceTxResponse?.gas).toBeDefined();
        expect(traceTxResponse?.structLogs?.length).toBeGreaterThan(0);

        // get transaction block information from tx hash
        const tx = await provider.getTransaction(call?.hash);
        expect(tx).toBeDefined();

        // test debug_traceBlockByHash
        const traceBlockResponse = await provider.send('debug_traceBlockByHash', [
            tx?.blockHash,
            { tracer: 'callTracer' },
        ]);

        expect(traceBlockResponse?.[0]?.gas).toBeDefined();
        expect(traceBlockResponse?.[0]?.gasUsed).toBeDefined();
        expect(traceBlockResponse?.[0]?.type).toBe('CALL');
        expect(traceBlockResponse?.[0]?.to).toBe(contractAddress.toLowerCase());
        expect(traceBlockResponse?.[0]?.calls?.length).toBe(1);
    }, 25000);
});
