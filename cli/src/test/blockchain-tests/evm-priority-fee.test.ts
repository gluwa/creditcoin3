import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise } from '../../lib';
import { evmAddressToSubstrateAddress } from '../../lib/evm/address';

// EVM priority fees are burned alongside the base fee; nothing is credited to COINBASE.
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
        const block = await provider.getBlock('latest');
        const baseFee = block!.baseFeePerGas!;
        const tx = await alith.sendTransaction({
            to: ethers.Wallet.createRandom().address,
            value: 1n,
            gasLimit: 21_000n,
            maxFeePerGas: baseFee * 2n + tip,
            maxPriorityFeePerGas: tip,
        });
        const receipt = (await tx.wait())!;
        expect(receipt.status).toBe(1);

        const minedBlock = (await provider.getBlock(receipt.blockNumber))!;
        const coinbase = evmAddressToSubstrateAddress(minedBlock.miner);
        const [before, after] = await Promise.all([
            api.at(minedBlock.parentHash).then((a) => a.query.system.account(coinbase)),
            api.at(minedBlock.hash!).then((a) => a.query.system.account(coinbase)),
        ]);
        const events = await api.at(minedBlock.hash!).then((a) => a.query.system.events());
        const burnedDebt = events
            .filter(({ event }) => api.events.balances.BurnedDebt.is(event))
            .map(({ event }) => (event.data[0] as any).toBigInt() as bigint);

        const gasUsed = receipt.gasUsed;
        const paidTip = gasUsed * (receipt.gasPrice - minedBlock.baseFeePerGas!);
        return {
            coinbaseDelta: after.data.free.toBigInt() - before.data.free.toBigInt(),
            baseBurn: gasUsed * minedBlock.baseFeePerGas!,
            paidTip,
            burnedDebt,
        };
    };

    test('a nonzero tip is burned and not credited to COINBASE', async () => {
        const tip = 1_000_000_000n;
        const { coinbaseDelta, baseBurn, paidTip, burnedDebt } = await sendWithTip(tip);

        expect(paidTip).toBe(21_000n * tip);
        expect(coinbaseDelta).toBe(0n);
        expect(burnedDebt).toContain(baseBurn);
        expect(burnedDebt).toContain(paidTip);
    }, 120_000);

    test('a zero tip burns only the base fee', async () => {
        const { coinbaseDelta, baseBurn, paidTip, burnedDebt } = await sendWithTip(0n);

        expect(paidTip).toBe(0n);
        expect(coinbaseDelta).toBe(0n);
        expect(burnedDebt).toContain(baseBurn);
    }, 120_000);
});
