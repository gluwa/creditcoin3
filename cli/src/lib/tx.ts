import { ISubmittableResult } from '@polkadot/types/types';

import { SubmittableExtrinsic } from '@polkadot/api/types';
import { AccountBalance, getBalance, toCTCString } from './balance';
import { ApiPromise, BN, KeyringPair } from '.';
import { CcKeyring, ProxyKeyring, delegateAddress, isProxy } from './account/keyring';

// WARNING: this function should not be used directly, use signSendAndWatchCcKeyring() instead!
async function internalSignSendAndWatch(
    tx: SubmittableExtrinsic<'promise', ISubmittableResult>,
    api: ApiPromise,
    signer: KeyringPair,
): Promise<TxResult> {
    return new Promise((resolve, reject) => {
        console.log('Sending transaction...');
        let maybeUnsub: (() => void) | undefined;
        const unsubAndResolve = (result: TxResult) => {
            if (maybeUnsub) {
                maybeUnsub();
            }
            resolve(result);
        };
        tx.signAndSend(signer, { nonce: -1 }, ({ status, dispatchError, events }) => {
            for (const { event } of events) {
                if (api.events.proxy.ProxyExecuted.is(event)) {
                    const [dispatchResult] = event.data;

                    if (dispatchResult.isErr) {
                        const proxyDispatchError = dispatchResult.asErr;
                        const { docs, name, section } = api.registry.findMetaError(proxyDispatchError.asModule);

                        const res = {
                            status: TxStatus.failed,
                            info: `Proxy Transaction failed: ${section}.${name}: ${docs.join(' ')}`,
                        };

                        unsubAndResolve(res);
                    }
                }
            }

            // Called every time the status changes
            if (status.isFinalized) {
                const result = {
                    status: TxStatus.ok,
                    info: `Transaction included at blockHash ${status.asFinalized.toString()}`,
                };
                unsubAndResolve(result);
            }
            if (dispatchError) {
                let blockHash: string | null = null;
                if (status.isInBlock) blockHash = status.asInBlock.toHex();

                if (dispatchError.isModule) {
                    // for module errors, the section is indexed, lookup
                    const decoded = api.registry.findMetaError(dispatchError.asModule);
                    const { docs, name, section } = decoded;
                    const error = `${section}.${name}: ${docs.join(' ')}`;
                    const result = {
                        status: TxStatus.failed,
                        info: `Transaction failed with error: "${error}" ${blockHash ? 'at block ' + blockHash : ''}`,
                    };
                    unsubAndResolve(result);
                } else {
                    // Other, CannotLookup, BadOrigin, no extra info
                    const result = {
                        status: TxStatus.failed,
                        info: `Transaction failed with error: "${dispatchError.toString()}" ${
                            blockHash ? 'at block ' + blockHash : ''
                        }`,
                    };
                    unsubAndResolve(result);
                }
            }
        })
            .then((unsub) => {
                maybeUnsub = unsub;
            })
            .catch((err) => {
                reject(err);
            });
    });
}

function proxyTx(tx: SubmittableExtrinsic<'promise', ISubmittableResult>, api: ApiPromise, keyring: ProxyKeyring) {
    return api.tx.proxy.proxy(keyring.proxiedAddress, null, tx.method);
}

export async function signSendAndWatchCcKeyring(
    tx: SubmittableExtrinsic<'promise', ISubmittableResult>,
    api: ApiPromise,
    keyring: CcKeyring,
) {
    switch (keyring.type) {
        case 'caller':
            return await internalSignSendAndWatch(tx, api, keyring.pair);
        case 'proxy': {
            const wrappedTx = proxyTx(tx, api, keyring);
            return await internalSignSendAndWatch(wrappedTx, api, keyring.pair);
        }
        default:
            const assertExhaustive = (_t: never) => {
                throw new Error(`Invalid keyring type`);
            };
            return assertExhaustive(keyring);
    }
}

// eslint-disable-next-line no-shadow
export enum TxStatus {
    ok,
    failed,
}

export interface TxResult {
    status: TxStatus;
    info: string;
}

export async function getTxFee(
    tx: SubmittableExtrinsic<'promise', ISubmittableResult>,
    callerAddress: string,
): Promise<BN> {
    const fee = await tx.paymentInfo(callerAddress);
    return fee.partialFee.toBn();
}

export function canPay(balance: AccountBalance, amount: BN, existentialDeposit = new BN(1)) {
    const availableBalance = balance.transferable;
    const availableAfter = availableBalance.sub(amount);
    return availableAfter.gte(existentialDeposit);
}

export async function requireKeyringHasSufficientFunds(
    tx: SubmittableExtrinsic<'promise', ISubmittableResult>,
    keyring: CcKeyring,
    api: ApiPromise,
    amount = new BN(0),
) {
    const address = delegateAddress(keyring);
    let totalCost = amount;

    // proxy inly pays transaction fees
    if (isProxy(keyring)) {
        const proxyAddress = keyring.pair.address;
        // construct the proxy transaction call in order to calculate fees more accurately
        const wrappedTx = proxyTx(tx, api, keyring as ProxyKeyring);

        const [proxyBalance, txFee] = await Promise.all([
            getBalance(proxyAddress, api),
            getTxFee(wrappedTx, proxyAddress),
        ]);

        if (!canPay(proxyBalance, txFee)) {
            console.error(
                `Caller ${proxyAddress} has insufficient funds to send the transaction (requires ${toCTCString(
                    txFee,
                )}); transaction cancelled.`,
            );
            process.exit(1);
        }
    } else {
        // when not using proxy caller needs amount + txFee
        const txFee = await getTxFee(tx, address);
        totalCost = amount.add(txFee);
    }

    const balance = await getBalance(address, api);

    if (!canPay(balance, totalCost)) {
        console.error(
            `Caller ${address} has insufficient funds to send the transaction (requires ${toCTCString(
                totalCost,
            )}); transaction cancelled.`,
        );
        process.exit(1);
    }
}
