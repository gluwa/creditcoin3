import { WebSocketProvider, ethers } from 'ethers';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { describeIf, dispatchErrorName, forElapsedBlocks, sudoInBlock } from '../../../utils';

// `revert_to` clears a chain's attestations in bounded batches: 40 rows in the call itself, the
// rest from a cursor drained 40 rows per block. Until the cursor is gone, another revert is
// refused and the leftover rows read as missing. The revert used to clear everything at once, up to
// a limit derived from the chain's intervals, and failed with `TooManyAttestations` past it.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'revertTo batched attestation cleanup', (): void => {
    let api: ApiPromise;
    let provider: WebSocketProvider;
    let root: KeyringPair;
    // A throwaway chain, loaded directly with more attestations than one batch clears.
    let chainKey: number;
    let revertedAt: number;

    // `MAX_ATTESTATIONS_CLEARED_PER_BLOCK` in pallets/attestation/src/clear_or_revert.rs.
    const BATCH = 40;
    const ROWS = 10 * BATCH;
    const digests = Array.from({ length: ROWS }, (_, i) => ethers.keccak256(ethers.toBeHex(i + 1, 32)));
    const CHECKPOINT_DIGEST = ethers.keccak256(ethers.toUtf8Bytes('checkpoint'));

    // The chain-info precompile (4051).
    const CHAIN_INFO = '0x0000000000000000000000000000000000000fd3';
    const chainInfo = new ethers.Interface([
        'function get_attestation_height_for_digest(uint64 chainKey, bytes32 digest) view returns (tuple(uint64 height, bool exists))',
    ]);

    const blockHash = async (blockNumber: number) => (await api.rpc.chain.getBlockHash(blockNumber)).toHex();
    const rowsAt = async (blockNumber: number) =>
        (await (await api.at(await blockHash(blockNumber))).query.attestation.attestations.keys(chainKey)).length;
    const cursorAt = async (blockNumber: number) =>
        (await (await api.at(await blockHash(blockNumber))).query.attestation.attestationClearingCursors(chainKey))
            .isSome;

    // What the chain-info precompile reports for `digest` at `blockNumber`.
    const attestationHeightAt = async (digest: string, blockNumber: number) => {
        const result = await provider.call({
            to: CHAIN_INFO,
            data: chainInfo.encodeFunctionData('get_attestation_height_for_digest', [chainKey, digest]),
            blockTag: blockNumber,
        });
        const [{ height, exists }] = chainInfo.decodeFunctionResult('get_attestation_height_for_digest', result);
        return { height: Number(height), exists: exists as boolean };
    };

    const revertTo = async () => {
        const result = await sudoInBlock(api, root, api.tx.attestation.revertTo(chainKey, 0));
        const blockNumber = (await api.rpc.chain.getHeader(result.blockHash)).number.toNumber();
        return { error: dispatchErrorName(api, result.innerError), blockNumber };
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const chainId = Date.now();
        const registration = await sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Batched cleanup ${chainId}`,
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

        // A checkpoint at height 0 to revert to, and the attestations committed on top of it.
        const attestation = (i: number) =>
            api.createType('AttestorPrimitivesSignedAttestation', {
                attestation: {
                    chainKey,
                    headerNumber: i + 1,
                    headerHash: ethers.ZeroHash,
                    root: ethers.ZeroHash,
                    prevDigest: null,
                },
                signature: new Uint8Array(96),
                attestors: [],
                continuityProof: { lowerEndpointDigest: ethers.ZeroHash, roots: [] },
            });
        const items: [string, string][] = digests.map((digest, i) => [
            api.query.attestation.attestations.key(chainKey, digest),
            attestation(i).toHex(),
        ]);
        items.push([
            api.query.attestation.checkpoints.key(chainKey, 0),
            api.createType('H256', CHECKPOINT_DIGEST).toHex(),
        ]);
        items.push([
            api.query.attestation.lastCheckpoint.key(chainKey),
            api
                .createType('AttestorPrimitivesAttestationCheckpoint', { blockNumber: 0, digest: CHECKPOINT_DIGEST })
                .toHex(),
        ]);
        const loaded = await sudoInBlock(api, root, api.tx.system.setStorage(items));
        expect(dispatchErrorName(api, loaded.innerError)).toBeNull();
        const loadedAt = (await api.rpc.chain.getHeader(loaded.blockHash)).number.toNumber();

        expect(await rowsAt(loadedAt)).toEqual(ROWS);
        // The rows are readable as attestations, so reading as missing later is the cleanup's doing.
        expect(await attestationHeightAt(digests[0], loadedAt)).toEqual({ height: 1, exists: true });
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
        await provider.destroy();
    });

    test('reverting clears one batch and leaves the rest to a cursor', async () => {
        const { error, blockNumber } = await revertTo();
        revertedAt = blockNumber;

        expect(error).toBeNull();
        expect(await rowsAt(revertedAt)).toEqual(ROWS - BATCH);
        expect(await cursorAt(revertedAt)).toBe(true);
    }, 60_000);

    test('another revert is refused while the cursor drains', async () => {
        const { error, blockNumber } = await revertTo();

        expect(await cursorAt(blockNumber)).toBe(true);
        expect(error).toEqual('attestation.TriedToRevertDuringOngoingReversion');
    }, 60_000);

    test('leftover attestations read as missing while the cursor drains', async () => {
        const at = await api.at(await blockHash(revertedAt));
        let leftover: string | undefined;
        for (const digest of digests) {
            if ((await at.query.attestation.attestations(chainKey, digest)).isSome) {
                leftover = digest;
                break;
            }
        }
        expect(leftover).toBeDefined();

        expect((await attestationHeightAt(leftover!, revertedAt)).exists).toBe(false);
    }, 60_000);

    test('the cursor drains one batch per block until no attestations are left', async () => {
        // Each block's `on_initialize` clears exactly one batch.
        await forElapsedBlocks(api, { minBlocks: 1 });
        expect(await rowsAt(revertedAt + 1)).toEqual(ROWS - 2 * BATCH);

        const deadline = Date.now() + 180_000;
        let blockNumber = revertedAt + 1;
        while (await cursorAt(blockNumber)) {
            if (Date.now() > deadline) {
                throw new Error(`cursor still set at block ${blockNumber}`);
            }
            await forElapsedBlocks(api, { minBlocks: 1 });
            blockNumber = (await api.rpc.chain.getHeader()).number.toNumber();
        }

        expect(await rowsAt(blockNumber)).toEqual(0);
        // 360 rows left after the revert take 9 blocks at 40 per block.
        expect(blockNumber - revertedAt).toBeGreaterThanOrEqual((ROWS - BATCH) / BATCH);
    }, 240_000);
});
