import { randomEvmAccount } from '../integration-tests/evmHelpers';
import { ALICE_NODE_URL, fundFromSudo, initAliceKeyring } from '../integration-tests/helpers';
import { deployContract } from './helpers';
import { Wallet, WebSocketProvider, ethers, parseEther } from 'ethers';

describe.only('EVM Tracing', (): void => {
    let provider: WebSocketProvider;
    let deployedContractAddress: string;
    let txHash: string;

    beforeAll(async () => {
        provider = new WebSocketProvider(ALICE_NODE_URL);

        const alith = new ethers.Wallet('0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133').connect(
            provider,
        );

        // deploy SendForYou Smart contract
        const contract = await deployContract('SendForYou', [], alith);
        deployedContractAddress = await contract.getAddress();

        // send funds to the contract
        const response = await alith.sendTransaction({
            to: deployedContractAddress,
            value: parseEther('50'),
        });
        await response.wait();

        // call contract method sendForMe to random address
        const call = await contract
            .getFunction('sendForMe')
            .call(contract, randomEvmAccount().address, parseEther('10'));

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
