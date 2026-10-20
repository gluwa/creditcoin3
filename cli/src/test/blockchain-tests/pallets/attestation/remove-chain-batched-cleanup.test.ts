import { SubmittableExtrinsic } from '@polkadot/api/types';
import { u8aToHex } from '@polkadot/util';
import { randomAsU8a } from '@polkadot/util-crypto';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { randomTestAccount } from '../../../integration-tests/helpers';
import { describeIf, dispatchErrorName, forElapsedBlocks, sudoInBlock } from '../../../utils';

// Removing a chain clears the attestor state it leaves behind (attestors, retired keys, BLS key
// owners, invulnerables) in bounded batches: 40 items in the removal itself, the rest from
// `on_initialize`. While that runs, nothing may write new state for the chain, so `attest` and
// `register_invulnerable` refuse it. Removal used to clear everything in one unbounded pass.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'removeChain batched cleanup', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let alice: KeyringPair;
    // A throwaway chain, loaded directly with more attestor state than one batch clears.
    let chainKey: number;
    let removedAt: number;

    // `MAX_REMOVAL_CLEANUP_ITEMS_PER_BLOCK` in pallets/attestation/src/clear_or_revert.rs.
    const BATCH = 40;
    const ROWS_PER_MAP = 5 * BATCH;

    const blockHash = async (blockNumber: number) => (await api.rpc.chain.getBlockHash(blockNumber)).toHex();
    const rowsAt = async (blockNumber: number) => {
        const at = await api.at(await blockHash(blockNumber));
        return {
            blsKeyOwners: (await at.query.attestation.blsKeyOwner.keys(chainKey)).length,
            invulnerables: (await at.query.attestation.invulnerables.keys(chainKey)).length,
        };
    };
    const cleanupPendingAt = async (blockNumber: number) =>
        (await (await api.at(await blockHash(blockNumber))).query.attestation.removedChainCleanup(chainKey)).isSome;
    const blockNumberOf = async (hash: string) => (await api.rpc.chain.getHeader(hash)).number.toNumber();

    // Signs and sends `tx` as `signer`, resolving once it's in a block with the block number and
    // the `section.Name` of its dispatch error, if any.
    const signedInBlock = (
        tx: SubmittableExtrinsic<'promise'>,
        signer: KeyringPair,
    ): Promise<{ error: string | null; blockNumber: number }> =>
        new Promise((resolve, reject) => {
            let unsubscribe: (() => void) | undefined;
            tx.signAndSend(signer, { nonce: -1 }, ({ dispatchError, status }) => {
                if (status.isInBlock || status.isFinalized) {
                    unsubscribe?.();
                    let error: string | null = null;
                    if (dispatchError?.isModule) {
                        const { section, name } = api.registry.findMetaError(dispatchError.asModule);
                        error = `${section}.${name}`;
                    } else if (dispatchError) {
                        error = dispatchError.toString();
                    }
                    const hash = (status.isInBlock ? status.asInBlock : status.asFinalized).toHex();
                    blockNumberOf(hash)
                        .then((blockNumber) => resolve({ error, blockNumber }))
                        .catch(reject);
                } else if (status.isDropped || status.isInvalid || status.isUsurped) {
                    unsubscribe?.();
                    reject(new Error(`extrinsic was not included: ${status.type}`));
                }
            })
                .then((unsub) => {
                    unsubscribe = unsub;
                })
                .catch((error) => reject(new Error(error)));
        });

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        const chainId = Date.now();
        const registration = await sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Removal cleanup ${chainId}`,
                null,
                null,
                null,
                null,
                null,
                null,
                'V1',
                null,
            ),
        );
        expect(dispatchErrorName(api, registration.innerError)).toBeNull();
        for (const { event } of registration.events) {
            if (api.events.supportedChains.ChainRegistered.is(event)) {
                chainKey = event.data.chainKey.toNumber();
            }
        }
        expect(chainKey).toBeDefined();

        // BLS key owners and invulnerables, as attestors registering on the chain would leave.
        const items: [string, string][] = [];
        for (let i = 0; i < ROWS_PER_MAP; i++) {
            const owner = api.createType('AccountId32', randomTestAccount().address).toHex();
            items.push([api.query.attestation.blsKeyOwner.key(chainKey, u8aToHex(randomAsU8a(48))), owner]);
            items.push([api.query.attestation.invulnerables.key(chainKey, randomTestAccount().address), '0x01']);
        }
        const loaded = await sudoInBlock(api, root, api.tx.system.setStorage(items));
        expect(dispatchErrorName(api, loaded.innerError)).toBeNull();

        expect(await rowsAt(await blockNumberOf(loaded.blockHash))).toEqual({
            blsKeyOwners: ROWS_PER_MAP,
            invulnerables: ROWS_PER_MAP,
        });
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('removal clears one batch and leaves the rest to on_initialize', async () => {
        const removal = await sudoInBlock(api, root, api.tx.supportedChains.removeChain(chainKey, true));
        expect(dispatchErrorName(api, removal.innerError)).toBeNull();
        removedAt = await blockNumberOf(removal.blockHash);

        // BLS key owners are cleared before invulnerables.
        expect(await rowsAt(removedAt)).toEqual({
            blsKeyOwners: ROWS_PER_MAP - BATCH,
            invulnerables: ROWS_PER_MAP,
        });
        expect(await cleanupPendingAt(removedAt)).toBe(true);
    }, 60_000);

    test('nothing can write new attestor state for the chain while it is cleaned up', async () => {
        const invulnerable = await sudoInBlock(
            api,
            root,
            api.tx.attestation.registerInvulnerable(chainKey, randomTestAccount().address),
        );
        expect(await cleanupPendingAt(await blockNumberOf(invulnerable.blockHash))).toBe(true);
        expect(dispatchErrorName(api, invulnerable.innerError)).toEqual('attestation.ChainNotSupported');

        // The chain is checked before anything about the caller or the key, so junk will do.
        const attest = await signedInBlock(
            api.tx.attestation.attest(chainKey, u8aToHex(randomAsU8a(48)), u8aToHex(randomAsU8a(96))),
            alice,
        );
        expect(attest.error).toEqual('attestation.ChainNotSupported');
    }, 60_000);

    test('the cleanup clears one batch per block until nothing is left', async () => {
        await forElapsedBlocks(api, { minBlocks: 1 });
        expect(await rowsAt(removedAt + 1)).toEqual({
            blsKeyOwners: ROWS_PER_MAP - 2 * BATCH,
            invulnerables: ROWS_PER_MAP,
        });

        const deadline = Date.now() + 180_000;
        let blockNumber = removedAt + 1;
        while (await cleanupPendingAt(blockNumber)) {
            if (Date.now() > deadline) {
                throw new Error(`removal cleanup still pending at block ${blockNumber}`);
            }
            await forElapsedBlocks(api, { minBlocks: 1 });
            blockNumber = (await api.rpc.chain.getHeader()).number.toNumber();
        }

        expect(await rowsAt(blockNumber)).toEqual({ blsKeyOwners: 0, invulnerables: 0 });
        // 360 items left after the removal take 9 blocks at 40 per block.
        expect(blockNumber - removedAt).toBeGreaterThanOrEqual((2 * ROWS_PER_MAP - BATCH) / BATCH);
    }, 240_000);
});
