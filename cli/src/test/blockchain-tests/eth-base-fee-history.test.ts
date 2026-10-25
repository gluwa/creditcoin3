import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { dispatchErrorName, forElapsedBlocks, sudoInBlock } from '../utils';

// A block's reported base fee must be the one it executed with, which its parent left in
// `baseFee.baseFeePerGas`, not the one left after it. Blocks, transactions and `eth_feeHistory`
// used to report the latter, which differs whenever the base fee moves.
describe('historical base fee reporting', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;
    // The block in which the test moves the base fee; it and the blocks after it each run with a
    // different base fee than they leave behind.
    let movedAt: number;

    const TIP = 1_000_000_000n;

    // The base fee stored after `blockNumber`: what block `blockNumber + 1` executes with.
    const baseFeeAfter = async (blockNumber: number): Promise<bigint> => {
        const hash = await api.rpc.chain.getBlockHash(blockNumber);
        return BigInt((await (await api.at(hash)).query.baseFee.baseFeePerGas()).toString());
    };
    const executionBaseFee = (blockNumber: number) => baseFeeAfter(blockNumber - 1);

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);

        // Raise the base fee by 1 gwei. Blocks then run with one value and leave a lower one, as
        // the fee decays back towards where it was.
        const current = await baseFeeAfter((await api.rpc.chain.getHeader()).number.toNumber());
        const { innerError, blockHash } = await sudoInBlock(
            api,
            root,
            api.tx.baseFee.setBaseFeePerGas(current + 1_000_000_000n),
        );
        expect(dispatchErrorName(api, innerError)).toBeNull();
        movedAt = (await api.rpc.chain.getHeader(blockHash)).number.toNumber();
        await forElapsedBlocks(api, { minBlocks: 3 });

        // Otherwise the assertions below couldn't tell the two values apart.
        expect(await baseFeeAfter(movedAt)).not.toEqual(await executionBaseFee(movedAt));
        expect(await baseFeeAfter(movedAt + 1)).not.toEqual(await executionBaseFee(movedAt + 1));
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test('eth_getBlockByNumber reports the base fee each block executed with', async () => {
        for (const blockNumber of [movedAt, movedAt + 1, movedAt + 2]) {
            const block = await provider.send('eth_getBlockByNumber', [ethers.toQuantity(blockNumber), false]);
            expect(BigInt(block.baseFeePerGas)).toEqual(await executionBaseFee(blockNumber));
        }
    }, 60_000);

    test("eth_feeHistory agrees, and ends with the next block's base fee", async () => {
        const newest = movedAt + 2;
        const history = await provider.send('eth_feeHistory', [ethers.toQuantity(3), ethers.toQuantity(newest), []]);

        expect(Number(history.oldestBlock)).toEqual(movedAt);
        const expected = [
            await executionBaseFee(movedAt),
            await executionBaseFee(movedAt + 1),
            await executionBaseFee(movedAt + 2),
            // One past the range: what the block after `newest` will run with.
            await baseFeeAfter(newest),
        ];
        expect(history.baseFeePerGas.map((fee: string) => BigInt(fee))).toEqual(expected);
    }, 60_000);

    test('a mined transaction reports the gas price it paid', async () => {
        const latest = (await provider.getBlock('latest'))!;
        const tx = await alith.sendTransaction({
            to: ethers.Wallet.createRandom().address,
            value: 1n,
            gasLimit: 21_000n,
            maxFeePerGas: latest.baseFeePerGas! * 2n + TIP,
            maxPriorityFeePerGas: TIP,
        });
        const receipt = (await tx.wait())!;
        expect(receipt.status).toBe(1);

        const paid = (await executionBaseFee(receipt.blockNumber)) + TIP;
        expect(receipt.gasPrice).toEqual(paid);
        const mined = await provider.send('eth_getTransactionByHash', [tx.hash]);
        expect(BigInt(mined.gasPrice)).toEqual(paid);
    }, 120_000);
});
