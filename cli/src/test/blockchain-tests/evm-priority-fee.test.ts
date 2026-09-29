import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise } from '../../lib';
import { substrateAddressToEvmAddress } from '../../lib/evm/address';
import { signSendAndWatchCcKeyring, TxStatus } from '../../lib/tx';
import { initAliceKeyring } from '../integration-tests/helpers';

// COINBASE is the block author's stash truncated to 20 bytes. EVM priority fees are credited
// there, and the stash can withdraw them with `evm.withdraw`.
describe('EVM priority fee', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let alith: ethers.Wallet;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);
    });

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    const sendWithTip = async (tip: bigint) => {
        const latest = (await provider.getBlock('latest'))!;
        const tx = await alith.sendTransaction({
            to: ethers.Wallet.createRandom().address,
            value: 1n,
            gasLimit: 21_000n,
            maxFeePerGas: latest.baseFeePerGas! * 2n + tip,
            maxPriorityFeePerGas: tip,
        });
        const receipt = (await tx.wait())!;
        expect(receipt.status).toBe(1);

        const block = (await provider.getBlock(receipt.blockNumber))!;
        // Ethereum block hashes are not Substrate block hashes; resolve blocks by number.
        const [parentHash, substrateHash] = await Promise.all([
            api.rpc.chain.getBlockHash(receipt.blockNumber - 1),
            api.rpc.chain.getBlockHash(receipt.blockNumber),
        ]);
        const author = (await api.derive.chain.getHeader(substrateHash)).author!.toString();
        // The base fee charged in a block is the one stored at its parent; the header's
        // `baseFeePerGas` can already reflect the next adjustment.
        const chargedBaseFee = BigInt((await (await api.at(parentHash)).query.baseFee.baseFeePerGas()).toString());

        // Every EVM transaction in the block tips the same COINBASE.
        const receipts = await Promise.all(block.transactions.map((h) => provider.getTransactionReceipt(h)));
        const blockTips = receipts.reduce((sum, r) => sum + r!.gasUsed * (r!.gasPrice - chargedBaseFee), 0n);

        const [before, after] = await Promise.all([
            provider.getBalance(block.miner, receipt.blockNumber - 1),
            provider.getBalance(block.miner, receipt.blockNumber),
        ]);
        return {
            author,
            miner: block.miner,
            coinbaseDelta: after - before,
            blockTips,
            paidTip: receipt.gasUsed * (receipt.gasPrice - chargedBaseFee),
        };
    };

    test('COINBASE is the block author stash EVM address', async () => {
        const { author, miner } = await sendWithTip(0n);

        expect(miner.toLowerCase()).toBe(substrateAddressToEvmAddress(author).toLowerCase());
    }, 120_000);

    test('a nonzero tip is credited to COINBASE', async () => {
        const tip = 1_000_000_000n;
        const { coinbaseDelta, blockTips, paidTip } = await sendWithTip(tip);

        expect(paidTip).toBe(21_000n * tip);
        expect(coinbaseDelta).toBe(blockTips);
        expect(coinbaseDelta >= paidTip).toBe(true);
    }, 120_000);

    test('a zero tip credits nothing extra to COINBASE', async () => {
        const { coinbaseDelta, blockTips, paidTip } = await sendWithTip(0n);

        expect(paidTip).toBe(0n);
        expect(coinbaseDelta).toBe(blockTips);
    }, 120_000);

    test('the author stash can withdraw its tips', async () => {
        const { author, miner } = await sendWithTip(1_000_000_000n);
        const alice = initAliceKeyring();
        // The dev chain is authored by Alice alone; nothing to check on other setups.
        if (author !== alice.address) return;

        const amount = await provider.getBalance(miner);
        expect(amount > 0n).toBe(true);
        const result = await signSendAndWatchCcKeyring(api.tx.evm.withdraw(miner, amount), api, {
            type: 'caller',
            pair: alice,
        });
        expect(result.status).toBe(TxStatus.ok);
    }, 120_000);
});
