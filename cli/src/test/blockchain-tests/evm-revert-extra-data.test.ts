import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise } from '../../lib';

// The `extra_data` the `Ethereum.Executed` event carries for a reverted transaction is capped at
// `ExtraDataLength` (30) bytes: the message of a well-formed `Error(string)`, otherwise the start
// of the raw revert data. Revert data that wasn't a well-formed `Error(string)`, or was 68 bytes
// or shorter, used to go into the event uncapped.
describe('EVM revert extra data', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;
    let reverter: string;

    // `ExtraDataLength` in runtime/src/lib.rs.
    const CAP = 30;

    // Runtime: revert with the calldata as revert data.
    //   36 CALLDATASIZE · 6000 PUSH1 0 · 6000 PUSH1 0 · 37 CALLDATACOPY · 36 CALLDATASIZE ·
    //   6000 PUSH1 0 · fd REVERT
    const RUNTIME = '366000600037366000fd';
    // Init: CODECOPY the 10-byte runtime that follows these 11 bytes to memory, then return it.
    //   600a PUSH1 10 · 80 DUP1 · 600b PUSH1 11 · 6000 PUSH1 0 · 39 CODECOPY · 6000 PUSH1 0 · f3 RETURN
    const INIT = '600a80600b6000396000f3';

    const abi = ethers.AbiCoder.defaultAbiCoder();
    const errorString = (message: string) =>
        ethers.concat([ethers.id('Error(string)').slice(0, 10), abi.encode(['string'], [message])]);

    // Reverts with `revertData` in a transaction and returns the `extra_data` of its `Executed` event.
    const extraDataFor = async (revertData: string): Promise<string> => {
        const tx = await alith.sendTransaction({ to: reverter, data: revertData, gasLimit: 200_000n });
        const receipt = (await provider.waitForTransaction(tx.hash))!;
        expect(receipt.status).toBe(0);

        const blockHash = await api.rpc.chain.getBlockHash(receipt.blockNumber);
        for (const { event } of await (await api.at(blockHash)).query.system.events()) {
            if (api.events.ethereum.Executed.is(event) && event.data.transactionHash.eq(tx.hash)) {
                return event.data.extraData.toHex();
            }
        }
        throw new Error(`no Executed event for ${tx.hash}`);
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);

        const receipt = (await (await alith.sendTransaction({ data: `0x${INIT}${RUNTIME}` })).wait())!;
        expect(receipt.status).toBe(1);
        reverter = receipt.contractAddress!;
        expect(await provider.getCode(reverter)).toEqual(`0x${RUNTIME}`);
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test('an Error(string) message is capped', async () => {
        const message = 'a'.repeat(100);
        expect(await extraDataFor(errorString(message))).toEqual(
            ethers.hexlify(ethers.toUtf8Bytes(message.slice(0, CAP))),
        );
    }, 120_000);

    test('an empty Error(string) gives no extra data', async () => {
        // 68 bytes of revert data, all of it encoding; used to be carried in full.
        expect(await extraDataFor(errorString(''))).toEqual('0x');
    }, 120_000);

    test('a custom error is cut to a prefix of the raw data', async () => {
        const customError = ethers.concat([
            ethers.id('Custom(uint256,uint256)').slice(0, 10),
            abi.encode(['uint256', 'uint256'], [1, 2]),
        ]);
        expect(ethers.dataLength(customError)).toEqual(68);
        expect(await extraDataFor(customError)).toEqual(ethers.dataSlice(customError, 0, CAP));
    }, 120_000);

    test('raw revert data is cut to a prefix', async () => {
        const raw = `0x${'ff'.repeat(64)}`;
        expect(await extraDataFor(raw)).toEqual(ethers.dataSlice(raw, 0, CAP));
    }, 120_000);

    test('an Error(string) that declares more message than it carries is cut to a prefix', async () => {
        // Selector, offset 32 and a length of 2^255 with no message after it.
        const truncated = ethers.concat([
            ethers.id('Error(string)').slice(0, 10),
            abi.encode(['uint256', 'uint256'], [32, 2n ** 255n]),
        ]);
        expect(await extraDataFor(truncated)).toEqual(ethers.dataSlice(truncated, 0, CAP));
    }, 120_000);
});
