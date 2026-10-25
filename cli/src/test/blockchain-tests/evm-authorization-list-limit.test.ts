import { SubmittableExtrinsic } from '@polkadot/api/types';
import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { parseAmount } from '../../commands/options';
import { evmAddressToSubstrateAddress, substrateAddressToEvmAddress } from '../../lib/evm/address';
import { randomTestAccount, TestAccount } from '../integration-tests/helpers';
import { dispatchErrorName, sudoInBlock } from '../utils';

// An EIP-7702 authorization list longer than 255 entries must be rejected up front, before any
// per-entry work such as recovering signatures, at every entry point. Only the size is checked
// first, so the entries below carry junk signatures.
describe('EIP-7702 authorization list size limit', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let root: KeyringPair;
    let chainId: number;
    // Signs `EVM.call`; its first 20 bytes are the EVM `source` (`EnsureAddressTruncated`).
    let caller: TestAccount;
    let source: string;

    // `MAX_AUTHORIZATION_LIST_SIZE` in frontier's primitives/evm.
    const MAX = 255;
    const JUNK = `0x${'11'.repeat(32)}`;

    // `eth_call` / `eth_estimateGas` request with `count` authorizations. Frontier's RPC takes
    // `chainId` as a number and `yParity` as a boolean.
    const rpcRequest = (count: number) => ({
        from: source,
        to: ethers.Wallet.createRandom().address,
        authorizationList: Array.from({ length: count }, () => ({
            chainId,
            address: ethers.Wallet.createRandom().address,
            nonce: '0x0',
            yParity: false,
            r: JUNK,
            s: JUNK,
        })),
    });

    // The JSON-RPC error a request with `count` authorizations fails with, or `null`. ethers wraps
    // these methods' errors as `CALL_EXCEPTION` and keeps the node's error under `info.error`.
    const rpcError = async (method: string, count: number): Promise<{ code?: number; message?: string } | null> => {
        try {
            await provider.send(method, [rpcRequest(count), 'latest']);
            return null;
        } catch (error: any) {
            return error.info?.error ?? error.error ?? { message: String(error.message ?? error) };
        }
    };

    // Signs and sends `EVM.call` with `count` authorizations as `caller`, resolving once it's in a
    // block with the `section.Name` of its dispatch error, if any.
    const evmCall = async (count: number): Promise<string | null> => {
        const baseFee = BigInt((await api.query.baseFee.baseFeePerGas()).toString());
        const authorizations = Array.from({ length: count }, () => ({
            chainId,
            address: ethers.Wallet.createRandom().address,
            nonce: 0,
            // The field name comes from the runtime's `MalleableTransactionSignature`.
            // eslint-disable-next-line @typescript-eslint/naming-convention
            signature: { oddYParity: false, r: JUNK, s: JUNK },
        }));
        const tx: SubmittableExtrinsic<'promise'> = api.tx.evm.call(
            source,
            ethers.Wallet.createRandom().address,
            '0x',
            0,
            10_000_000n,
            baseFee * 2n,
            null,
            null,
            [],
            authorizations,
        );
        return new Promise((resolve, reject) => {
            let unsubscribe: (() => void) | undefined;
            tx.signAndSend(caller.keyring, { nonce: -1 }, ({ dispatchError, status }) => {
                if (status.isInBlock || status.isFinalized) {
                    unsubscribe?.();
                    if (!dispatchError) {
                        resolve(null);
                    } else if (dispatchError.isModule) {
                        const { section, name } = api.registry.findMetaError(dispatchError.asModule);
                        resolve(`${section}.${name}`);
                    } else {
                        resolve(dispatchError.toString());
                    }
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
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        chainId = Number((await provider.getNetwork()).chainId);

        // Fund the signer for the extrinsic fee, and the account the EVM maps `source` to for gas.
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
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test.each(['eth_call', 'eth_estimateGas'])(
        '%s rejects more than 255 authorizations',
        async (method) => {
            const error = await rpcError(method, MAX + 1);
            expect(error?.code).toEqual(-32602);
            expect(error?.message).toContain(`authorization list too large: ${MAX + 1} > ${MAX}`);

            // At the limit the size check passes; whatever happens next isn't this rejection.
            expect((await rpcError(method, MAX))?.message ?? '').not.toContain('authorization list too large');
        },
        60_000,
    );

    test('EVM.call rejects more than 255 authorizations', async () => {
        expect(await evmCall(MAX + 1)).toEqual('evm.AuthorizationListTooLarge');
        expect(await evmCall(MAX)).not.toEqual('evm.AuthorizationListTooLarge');
    }, 120_000);
});
