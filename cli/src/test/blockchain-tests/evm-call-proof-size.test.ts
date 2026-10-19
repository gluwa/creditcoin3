import { SubmittableExtrinsic } from '@polkadot/api/types';
import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise, KeyringPair } from '../../lib';
import type { EventRecord } from '../../lib';
import { parseAmount } from '../../commands/options';
import { evmAddressToSubstrateAddress, substrateAddressToEvmAddress } from '../../lib/evm/address';
import { randomTestAccount, TestAccount } from '../integration-tests/helpers';
import { dispatchErrorName, sudoInBlock } from '../utils';

// The `EVM.call` extrinsic must be bound by the proof size its gas limit buys, like an Ethereum
// transaction: reading code it can't afford fails the call, and an encoded call that alone
// exceeds the limit is rejected. It used to run with no proof-size limit at all.
//
// TODO: remove this test once the PoV component of gas cost is removed. Creditcoin3 isn't a
// parachain and never uses the PoV, so this limit only exists while that component does.
describe('EVM.call proof-size limit', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let root: KeyringPair;
    // Signs `EVM.call`; its first 20 bytes are the EVM `source` (`EnsureAddressTruncated`).
    let caller: TestAccount;
    let source: string;
    // A contract with 24,000 bytes of code: far more proof size than 21,000 gas buys.
    let bigContract: string;

    // `GasLimitPovSizeRatio` in runtime/src/lib.rs: gas per byte of proof size.
    const GAS_PER_PROOF_BYTE = 14n;
    const VALUE = 777n;

    // Init code that returns 24,000 zero bytes (0x5dc0) of memory as the runtime code.
    //   615dc0 PUSH2 24000 · 6000 PUSH1 0 · f3 RETURN
    const BIG_CODE_INIT = '0x615dc06000f3';

    // Signs and sends `tx` as `caller`, resolving once it's in a block with its events and the
    // `section.Name` of its dispatch error, if any.
    const send = (tx: SubmittableExtrinsic<'promise'>): Promise<{ error: string | null; events: EventRecord[] }> =>
        new Promise((resolve, reject) => {
            let unsubscribe: (() => void) | undefined;
            tx.signAndSend(caller.keyring, { nonce: -1 }, ({ dispatchError, events, status }) => {
                if (status.isInBlock || status.isFinalized) {
                    unsubscribe?.();
                    let error: string | null = null;
                    if (dispatchError) {
                        error = dispatchError.isModule
                            ? (({ section, name }) => `${section}.${name}`)(
                                  api.registry.findMetaError(dispatchError.asModule),
                              )
                            : dispatchError.toString();
                    }
                    resolve({ error, events });
                } else if (status.isDropped || status.isInvalid || status.isUsurped) {
                    unsubscribe?.();
                    reject(new Error(`EVM.call was not included: ${status.type}`));
                }
            })
                .then((unsub) => {
                    unsubscribe = unsub;
                })
                .catch((error) => reject(new Error(error)));
        });

    const evmCall = async (target: string, input: string, gasLimit: bigint) => {
        const baseFee = BigInt((await api.query.baseFee.baseFeePerGas()).toString());
        return send(api.tx.evm.call(source, target, input, VALUE, gasLimit, baseFee * 2n, null, null, [], []));
    };

    const evmEvents = (events: EventRecord[], address: string) =>
        events
            .map(({ event }) => event)
            .filter(
                (event) =>
                    (api.events.evm.Executed.is(event) || api.events.evm.ExecutedFailed.is(event)) &&
                    event.data.address.eq(address),
            )
            .map((event) => event.method);

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // Fund the signer for the extrinsic fee, and the account the EVM maps `source` to for the
        // value and gas.
        caller = randomTestAccount();
        source = substrateAddressToEvmAddress(caller.address);
        const funding = await sudoInBlock(
            api,
            root,
            api.tx.utility.batchAll([
                api.tx.balances.forceSetBalance(caller.address, parseAmount('100').toString()),
                api.tx.balances.forceSetBalance(evmAddressToSubstrateAddress(source), parseAmount('100').toString()),
            ]),
        );
        expect(dispatchErrorName(api, funding.innerError)).toBeNull();

        const alith = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice'), provider);
        const receipt = (await (await alith.sendTransaction({ data: BIG_CODE_INIT, gasLimit: 15_000_000n })).wait())!;
        expect(receipt.status).toBe(1);
        bigContract = receipt.contractAddress!;
        expect(ethers.dataLength(await provider.getCode(bigContract))).toEqual(24_000);
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test("a call that can't afford to read its target's code fails and moves no value", async () => {
        const before = await provider.getBalance(bigContract);

        // 21,000 gas covers a plain transfer, but buys 1,500 bytes of proof size, not 24,000.
        const { error, events } = await evmCall(bigContract, '0x', 21_000n);

        expect(error).toBeNull();
        expect(evmEvents(events, bigContract)).toEqual(['ExecutedFailed']);
        expect(await provider.getBalance(bigContract)).toEqual(before);
    }, 60_000);

    test('an encoded call larger than the proof size its gas limit buys is rejected', async () => {
        const target = ethers.Wallet.createRandom().address;
        const input = ethers.hexlify(new Uint8Array(10_000));

        // Enough gas for the call data, floor cost included, but it buys fewer bytes of proof size
        // than the encoded call takes up.
        const tooLow = 21_000n + 10n * 10_000n;
        expect(tooLow / GAS_PER_PROOF_BYTE < 10_000n).toBe(true);
        expect((await evmCall(target, input, tooLow)).error).toEqual('evm.GasLimitTooLow');

        // With room for the encoded call and the reads it makes, the same call goes through.
        const enough = GAS_PER_PROOF_BYTE * 12_000n;
        const { error, events } = await evmCall(target, input, enough);
        expect(error).toBeNull();
        expect(evmEvents(events, target)).toEqual(['Executed']);
    }, 120_000);
});
