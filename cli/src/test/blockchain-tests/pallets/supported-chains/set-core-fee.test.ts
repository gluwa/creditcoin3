import type { SubmittableExtrinsic } from '@polkadot/api/types';
import { U64 } from '@polkadot/types-codec';
import type { ISubmittableResult } from '@polkadot/types/types';
import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import type { Balance, DispatchError, EventRecord } from '../../../../lib';
import { describeIf, forElapsedBlocks } from '../../../utils';

/// Outcome of an extrinsic wrapped in `sudo.sudo`, once it made it into a block.
type SudoOutcome = {
    /// Transaction fee withdrawn from the sudo account.
    fee: bigint;
    events: EventRecord[];
    /// Name of the pallet error the *wrapped* call failed with, `undefined` when it succeeded.
    error?: string;
};

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'SetCoreFee', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let chainKey: U64;

    const chainId = Date.now();
    const chainName = `Test Chain ${chainId}`;
    const encoding = 'V1';

    // A chain key which is never registered, to exercise the ChainNotSupported guard
    const unsupportedChainKey = 42_732;

    // Core-fee amounts are 18-decimal EVM values: the Outbox charges them as `msg.value` (native)
    // or pulls them with `transferFrom` (ERC20), so they are wei-denominated, not microunits.
    const oneCtc = '1000000000000000000';
    const halfCtc = '500000000000000000';

    // 20-byte ERC20 address on Creditcoin's EVM, standing in for the future attestcoin token.
    // Repeated-nibble addresses keep `H160.toString()` comparisons free of checksum casing.
    const erc20Token = '0x2222222222222222222222222222222222222222';
    const zeroToken = '0x0000000000000000000000000000000000000000';

    // Decodes the result of the call wrapped in `sudo.sudo`. The outer sudo extrinsic dispatches
    // successfully even when the wrapped call fails, so a failure is only visible through the
    // `sudo.Sudid` event. Returns the pallet error name, e.g. 'ChainNotSupported'.
    const wrappedCallError = (events: EventRecord[]): string | undefined => {
        for (const { event } of events) {
            if (api.events.sudo.Sudid.is(event)) {
                const [sudoResult] = event.data;

                if (sudoResult.isErr) {
                    const failure = sudoResult.asErr;

                    return failure.isModule ? api.registry.findMetaError(failure.asModule).name : failure.type;
                }
            }
        }

        return undefined;
    };

    // Sends `call` through sudo and resolves once it is in a block. Sibling suites reach for
    // `extractFee` here, but these tests also assert on the emitted event and on failures of the
    // *wrapped* call, neither of which `extractFee` surfaces.
    const submitSudo = async (call: SubmittableExtrinsic<'promise', ISubmittableResult>): Promise<SudoOutcome> => {
        const nonce = await api.rpc.system.accountNextIndex(root.address);

        return new Promise<SudoOutcome>((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(call)
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    // The outer sudo extrinsic must always dispatch fine, //Alice is the sudo key
                    if (dispatchError) {
                        reject(new Error(`sudo.sudo failed: ${dispatchError.toString()}`));
                        return;
                    }

                    if (!status.isInBlock) {
                        return;
                    }

                    const unsub = await unsubscribe;

                    if (!unsub) {
                        reject(new Error('Subscription failed'));
                        return;
                    }

                    unsub();

                    const balancesWithdraw = events.find(({ event: { method, section } }) => {
                        return section === 'balances' && method === 'Withdraw';
                    });

                    if (!balancesWithdraw) {
                        reject(new Error("Fee wasn't found"));
                        return;
                    }

                    resolve({
                        fee: (balancesWithdraw.event.data[1] as Balance).toBigInt(),
                        events,
                        error: wrappedCallError(events),
                    });
                })
                .catch((reason) => reject(new Error(reason)));
        });
    };

    const coreFeeSetEvent = (events: EventRecord[]) => {
        for (const { event } of events) {
            if (api.events.supportedChains.CoreFeeSet.is(event)) {
                return event.data;
            }
        }

        return undefined;
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const nonce = await api.rpc.system.accountNextIndex(root.address);

        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(
                    chainId,
                    chainName,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    encoding,
                    null,
                ),
            )
            .signAndSend(root, { nonce });

        await forElapsedBlocks(api);

        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
    }, 30_000);

    afterAll(async () => {
        // Drop the chain registered for this suite so the supported-chain set is left as genesis
        // had it: precompiles/chain-info.test.ts asserts on the number of supported chains.
        if (chainKey !== undefined) {
            await submitSudo(api.tx.supportedChains.removeChain(chainKey, true));
        }

        await api.disconnect();
    }, 30_000);

    it('sets a core fee denominated in native CTC, fee is min 0.01 CTC', async (): Promise<void> => {
        const { fee, events, error } = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, null, oneCtc));

        expect(error).toBeUndefined();
        expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);

        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        // A native-CTC fee is stored as `token: None`; the extrinsic rejects the zero address
        expect(stored.unwrap().token.isNone).toEqual(true);
        expect(stored.unwrap().amount.toString()).toEqual(oneCtc);

        const emitted = coreFeeSetEvent(events);

        expect(emitted).toBeDefined();
        expect(emitted?.chainKey.toString()).toEqual(chainKey.toString());
        expect(emitted?.token.isNone).toEqual(true);
        expect(emitted?.amount.toString()).toEqual(oneCtc);
    }, 30_000);

    it('sets a core fee denominated in an ERC20', async (): Promise<void> => {
        const { events, error } = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, erc20Token, halfCtc));

        expect(error).toBeUndefined();

        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().token.isSome).toEqual(true);
        expect(stored.unwrap().token.unwrap().toString()).toEqual(erc20Token);
        expect(stored.unwrap().amount.toString()).toEqual(halfCtc);

        const emitted = coreFeeSetEvent(events);

        expect(emitted).toBeDefined();
        expect(emitted?.chainKey.toString()).toEqual(chainKey.toString());
        expect(emitted?.token.isSome).toEqual(true);
        expect(emitted?.token.unwrap().toString()).toEqual(erc20Token);
        expect(emitted?.amount.toString()).toEqual(halfCtc);
    }, 30_000);

    it('overwrites an existing core fee, last write wins', async (): Promise<void> => {
        const first = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, erc20Token, oneCtc));

        expect(first.error).toBeUndefined();

        // Switching back to a native-denominated fee also has to flip `token` from Some to None,
        // which is the governance switch the precompile is meant to serve without a redeploy.
        const second = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, null, halfCtc));

        expect(second.error).toBeUndefined();

        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().token.isNone).toEqual(true);
        expect(stored.unwrap().amount.toString()).toEqual(halfCtc);
    }, 60_000);

    it('is rejected for a non-operator origin', async (): Promise<void> => {
        // The Operators membership is empty on a dev chain, so every signed origin - even //Bob,
        // who is endowed at genesis - fails the EnsureRootOrOperators check with BadOrigin.
        const bob: KeyringPair = (global as any).CREDITCOIN_CREATE_SIGNER('bob');
        const nonce = await api.rpc.system.accountNextIndex(bob.address);
        const before = await api.query.supportedChains.coreFees(chainKey);

        const failure = await new Promise<DispatchError>((resolve, reject): void => {
            const unsubscribe = api.tx.supportedChains
                .setCoreFee(chainKey, erc20Token, oneCtc)
                .signAndSend(bob, { nonce }, async ({ dispatchError, status }) => {
                    if (!dispatchError && !status.isInBlock) {
                        return;
                    }

                    const unsub = await unsubscribe;

                    if (unsub) {
                        unsub();
                    }

                    if (dispatchError) {
                        resolve(dispatchError);
                    } else {
                        reject(new Error('set_core_fee unexpectedly succeeded for a non-operator origin'));
                    }
                })
                .catch((reason) => reject(new Error(reason)));
        });

        expect(failure.isBadOrigin).toEqual(true);

        // The rejected call must not have touched the stored fee
        const after = await api.query.supportedChains.coreFees(chainKey);

        expect(after.toHex()).toEqual(before.toHex());
    }, 30_000);

    it('rejects a zero core-fee token address', async (): Promise<void> => {
        // address(0) means "native currency" on the EVM side, which this config expresses as
        // `token: None`, so a zero ERC20 address is always a misconfiguration.
        const { error } = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, zeroToken, oneCtc));

        expect(error).toEqual('ZeroCoreFeeToken');
    }, 30_000);

    it('rejects an unsupported chain key', async (): Promise<void> => {
        const { error } = await submitSudo(api.tx.supportedChains.setCoreFee(unsupportedChainKey, null, oneCtc));

        expect(error).toEqual('ChainNotSupported');

        const stored = await api.query.supportedChains.coreFees(unsupportedChainKey);

        expect(stored.isNone).toEqual(true);
    }, 30_000);
});
