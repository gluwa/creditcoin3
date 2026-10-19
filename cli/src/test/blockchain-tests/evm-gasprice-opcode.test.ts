import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise } from '../../lib';

// GASPRICE (`tx.gasprice`) must be the price per gas the sender pays: the effective price of a
// real transaction, or for `eth_call` the price implied by the request's fee fields. It used to be
// the block base fee in every case.
describe('EVM GASPRICE opcode', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;
    let probe: string;

    // Runtime: SSTORE(0, GASPRICE), then return GASPRICE as a 32-byte word.
    //   3a GASPRICE · 80 DUP1 · 6000 PUSH1 0 · 55 SSTORE · 6000 PUSH1 0 · 52 MSTORE ·
    //   6020 PUSH1 32 · 6000 PUSH1 0 · f3 RETURN
    const RUNTIME = '3a8060005560005260206000f3';
    // Init: CODECOPY the 13-byte runtime that follows these 11 bytes to memory, then return it.
    //   600d PUSH1 13 · 80 DUP1 · 600b PUSH1 11 · 6000 PUSH1 0 · 39 CODECOPY · 6000 PUSH1 0 · f3 RETURN
    const INIT = '600d80600b6000396000f3';

    const TIP = 1_000_000_000n;

    // The base fee stored after `blockNumber`: what the next block charges, and what an
    // `eth_call` against `blockNumber` runs with.
    const baseFeeAfter = async (blockNumber: number): Promise<bigint> => {
        const hash = await api.rpc.chain.getBlockHash(blockNumber);
        return BigInt((await (await api.at(hash)).query.baseFee.baseFeePerGas()).toString());
    };

    const send = async (tx: ethers.TransactionRequest) => {
        const receipt = (await (await alith.sendTransaction({ to: probe, gasLimit: 100_000n, ...tx })).wait())!;
        expect(receipt.status).toBe(1);
        const stored = BigInt(await provider.getStorage(probe, 0, receipt.blockNumber));
        return { receipt, stored, chargedBaseFee: await baseFeeAfter(receipt.blockNumber - 1) };
    };

    // `eth_call` against the latest block, with fee fields built from the base fee it runs with.
    const call = async (feeFields: (baseFee: bigint) => Record<string, bigint>) => {
        const blockNumber = await provider.getBlockNumber();
        const baseFee = await baseFeeAfter(blockNumber);
        const request: Record<string, string> = { from: alith.address, to: probe, gas: ethers.toQuantity(100_000n) };
        for (const [field, value] of Object.entries(feeFields(baseFee))) {
            request[field] = ethers.toQuantity(value);
        }
        const result = await provider.send('eth_call', [request, ethers.toQuantity(blockNumber)]);
        return { gasPrice: BigInt(result), baseFee };
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);

        const receipt = (await (await alith.sendTransaction({ data: `0x${INIT}${RUNTIME}` })).wait())!;
        expect(receipt.status).toBe(1);
        probe = receipt.contractAddress!;
        expect(await provider.getCode(probe)).toBe(`0x${RUNTIME}`);
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test('a type-2 transaction sees its effective gas price', async () => {
        const latest = (await provider.getBlock('latest'))!;
        const { receipt, stored, chargedBaseFee } = await send({
            maxFeePerGas: latest.baseFeePerGas! * 2n + TIP,
            maxPriorityFeePerGas: TIP,
        });

        expect(stored).toBe(chargedBaseFee + TIP);
        expect(stored).toBe(receipt.gasPrice);
    }, 120_000);

    test('a legacy transaction sees its gas price', async () => {
        const latest = (await provider.getBlock('latest'))!;
        const gasPrice = latest.baseFeePerGas! * 2n;
        const { receipt, stored, chargedBaseFee } = await send({ type: 0, gasPrice });

        expect(stored).toBe(gasPrice);
        expect(stored).toBe(receipt.gasPrice);
        expect(stored > chargedBaseFee).toBe(true);
    }, 120_000);

    test('eth_call sees the price implied by its EIP-1559 fee fields', async () => {
        const { gasPrice, baseFee } = await call((base) => ({
            maxFeePerGas: base * 2n + TIP,
            maxPriorityFeePerGas: TIP,
        }));

        expect(gasPrice).toBe(baseFee + TIP);
    }, 60_000);

    test('eth_call caps the tip at maxFeePerGas', async () => {
        const headroom = TIP / 2n;
        const { gasPrice, baseFee } = await call((base) => ({
            maxFeePerGas: base + headroom,
            maxPriorityFeePerGas: TIP,
        }));

        expect(gasPrice).toBe(baseFee + headroom);
    }, 60_000);

    test('eth_call without fee fields sees the base fee', async () => {
        const { gasPrice, baseFee } = await call(() => ({}));

        expect(gasPrice).toBe(baseFee);
    }, 60_000);
});
