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

    // The core fee is always denominated in attestcoin, so amounts are ATTEST wei. There is no
    // token parameter: the Outbox pulls the fee with transferFrom on its configured ATTEST token
    // and has no native-currency path, so a configurable denomination could only ever disagree with
    // what is actually charged.
    const oneAttest = '1000000000000000000';
    const halfAttest = '500000000000000000';

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

    it('sets a core fee, fee is min 0.01 CTC', async (): Promise<void> => {
        const { fee, events, error } = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, oneAttest));

        expect(error).toBeUndefined();
        expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);

        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().amount.toString()).toEqual(oneAttest);

        const emitted = coreFeeSetEvent(events);

        expect(emitted).toBeDefined();
        expect(emitted?.chainKey.toString()).toEqual(chainKey.toString());
        expect(emitted?.amount.toString()).toEqual(oneAttest);
    }, 30_000);

    it('overwrites an existing core fee, last write wins', async (): Promise<void> => {
        const first = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, oneAttest));

        expect(first.error).toBeUndefined();

        const second = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, halfAttest));

        expect(second.error).toBeUndefined();

        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().amount.toString()).toEqual(halfAttest);
    }, 60_000);

    it('accepts a zero amount, which disables the fee', async (): Promise<void> => {
        const { error } = await submitSudo(api.tx.supportedChains.setCoreFee(chainKey, 0));

        expect(error).toBeUndefined();

        // The entry is kept with a zero amount; the precompile reports 0, which the Outbox treats
        // as "charge nothing" — the same outcome as no entry at all.
        const stored = await api.query.supportedChains.coreFees(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().amount.toString()).toEqual('0');
    }, 30_000);

    it('is rejected for a non-operator origin', async (): Promise<void> => {
        // The Operators membership is empty on a dev chain, so every signed origin - even //Bob,
        // who is endowed at genesis - fails the EnsureRootOrOperators check with BadOrigin.
        const bob: KeyringPair = (global as any).CREDITCOIN_CREATE_SIGNER('bob');
        const nonce = await api.rpc.system.accountNextIndex(bob.address);
        const before = await api.query.supportedChains.coreFees(chainKey);

        const failure = await new Promise<DispatchError>((resolve, reject): void => {
            const unsubscribe = api.tx.supportedChains
                .setCoreFee(chainKey, oneAttest)
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

    it('rejects an unsupported chain key', async (): Promise<void> => {
        const { error } = await submitSudo(api.tx.supportedChains.setCoreFee(unsupportedChainKey, oneAttest));

        expect(error).toEqual('ChainNotSupported');

        const stored = await api.query.supportedChains.coreFees(unsupportedChainKey);

        expect(stored.isNone).toEqual(true);
    }, 30_000);
});
