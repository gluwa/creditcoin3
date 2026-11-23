import { SubmittableExtrinsic } from '@polkadot/api/types';
import { bnToU8a, u8aConcat, u8aToHex } from '@polkadot/util';
import { keccakAsU8a, randomAsU8a } from '@polkadot/util-crypto';
import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { proofOfPossessionMessage } from '../../../../lib/attestor/proof-of-possession';
import { parseAmount } from '../../../../commands/options';
import { randomTestAccount, TestAccount } from '../../../integration-tests/helpers';
import { describeIf, dispatchErrorName, sudoInBlock } from '../../../utils';

// Regression test: honest attestors paying fees for lost submission races.
//
// Since #1445 a commit's continuity proof must extend the chain's *latest* digest. Attestors
// routinely form quorums for several heights on the same anchor, so once one lands the others can
// only fail. `PrevalidateAttestationCommit` must reject those rivals as `Stale` before fees, in the
// pool and during block building. If it lets them through, they fail in dispatch with
// `InvalidAttestationContinuityProofTail` and the attestor pays the full fee.
//
// If this test fails, don't loosen it: the extension has regressed. See the maintenance notes at
// the top of `pallets/attestation/src/extensions.rs`.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'commitAttestation on a superseded anchor', (): void => {
    const INTERVAL = 10;
    const GENESIS = 0;

    let api: ApiPromise;
    let root: KeyringPair;
    let alice: KeyringPair;
    // The only active attestor on a throwaway chain, so its signature alone is a quorum.
    let attestor: TestAccount;
    let blsKey: WasmPrivateKey;
    let chainKey: number;

    type Outcome = 'success' | 'failed' | 'dropped';

    // What became of a submitted extrinsic: landed and succeeded, landed and failed in dispatch
    // (so it paid its fee), or dropped by the pool or block builder (no fee).
    const send = (tx: SubmittableExtrinsic<'promise'>, signer: KeyringPair, nonce = -1): Promise<Outcome> =>
        new Promise((resolve, reject) => {
            let unsubscribe: (() => void) | undefined;
            const done = (outcome: Outcome) => {
                unsubscribe?.();
                resolve(outcome);
            };
            tx.signAndSend(signer, { nonce }, ({ dispatchError, status }) => {
                if (status.isInBlock) {
                    done(dispatchError ? 'failed' : 'success');
                } else if (status.isInvalid || status.isDropped || status.isUsurped) {
                    done('dropped');
                }
            })
                .then((unsub) => {
                    unsubscribe = unsub;
                })
                .catch((error) => reject(new Error(error)));
        });

    // `compute_digest_for` in primitives/attestor: keccak(be64(height) || root || prev_digest?).
    const digestFor = (height: number, blockRoot: Uint8Array, prev: Uint8Array | null): Uint8Array =>
        keccakAsU8a(u8aConcat(bnToU8a(height, { bitLength: 64, isLe: false }), blockRoot, prev ?? new Uint8Array()));

    // A signed commit for `height` whose continuity proof starts at `anchor`, the digest of the
    // attestation at `anchorHeight`, and covers every block in between with arbitrary roots.
    const commitOn = (height: number, anchorHeight: number, anchor: Uint8Array | null) => {
        const roots = anchor ? Array.from({ length: height - anchorHeight - 1 }, () => randomAsU8a(32)) : [];
        let prevDigest = anchor;
        roots.forEach((proofRoot, i) => {
            prevDigest = digestFor(anchorHeight + 1 + i, proofRoot, prevDigest);
        });
        const headerHash = randomAsU8a(32);
        const attestationRoot = randomAsU8a(32);
        // The BLS-signed bytes, `AttestationData::serialize`:
        // le64(chain_key) || le64(height) || header_hash || root || prev_digest?.
        const message = u8aConcat(
            bnToU8a(chainKey, { bitLength: 64, isLe: true }),
            bnToU8a(height, { bitLength: 64, isLe: true }),
            headerHash,
            attestationRoot,
            prevDigest ?? new Uint8Array(),
        );
        const tx = api.tx.attestation.commitAttestation({
            // Hex, not bytes: polkadot.js reads a `Uint8Array` for an `Option` as already encoded.
            attestation: {
                chainKey,
                headerNumber: height,
                headerHash,
                root: attestationRoot,
                prevDigest: prevDigest && u8aToHex(prevDigest),
            },
            signature: blsKey.sign(message).as_bytes(),
            attestors: [attestor.address],
            continuityProof: { lowerEndpointDigest: anchor ?? new Uint8Array(32), roots },
        });
        return { tx, digest: digestFor(height, attestationRoot, prevDigest) };
    };

    const lastDigest = async () => (await api.query.attestation.lastDigest(chainKey)).unwrap()[1].toHex();
    const freeBalance = async () => (await api.query.system.account(attestor.address)).data.free.toBigInt();

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
                `Stale anchor ${chainId}`,
                null,
                INTERVAL,
                null,
                null,
                null,
                GENESIS,
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

        attestor = randomTestAccount();
        const funding = await sudoInBlock(
            api,
            root,
            api.tx.balances.forceSetBalance(attestor.address, parseAmount('1000').toString()),
        );
        expect(dispatchErrorName(api, funding.innerError)).toBeNull();

        // NOTE: Alice is the STASH for the attestor
        expect(await send(api.tx.attestation.registerAttestor(chainKey, attestor.address), alice)).toEqual('success');

        blsKey = WasmPrivateKey.generate(new TextEncoder().encode(attestor.secret));
        const blsPublicKey = blsKey.public_key().as_bytes();
        const proofOfPossession = blsKey
            .sign(proofOfPossessionMessage(chainKey, attestor.address, blsPublicKey))
            .as_bytes();
        expect(
            await send(api.tx.attestation.attest(chainKey, blsPublicKey, proofOfPossession), attestor.keyring),
        ).toEqual('success');

        const election = await sudoInBlock(api, root, api.tx.attestation.forceElection(chainKey));
        expect(dispatchErrorName(api, election.innerError)).toBeNull();
        const active = await api.query.attestation.activeAttestors(chainKey);
        expect(active.map((a) => a.toString())).toEqual([attestor.address]);
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('a rival quorum on a superseded anchor is dropped without a fee', async () => {
        const genesis = commitOn(GENESIS, GENESIS, null);
        expect(await send(genesis.tx, attestor.keyring)).toEqual('success');
        expect(await lastDigest()).toEqual(u8aToHex(genesis.digest));

        // Two quorums on the same anchor, as attestors produce every round on mainnet. Sent
        // back to back so they normally share a block; either way the rival must not pay.
        const lower = commitOn(INTERVAL, GENESIS, genesis.digest);
        const rival = commitOn(2 * INTERVAL, GENESIS, genesis.digest);
        const nonce = (await api.rpc.system.accountNextIndex(attestor.address)).toNumber();
        const balanceBefore = await freeBalance();
        await rival.tx.signAsync(attestor.keyring, { nonce: nonce + 1 });

        // The pool doesn't reliably emit a terminal status for a transaction that turns invalid
        // during block building or revalidation, so watch the rival only for the failure we care
        // about: landing in a block, which is what costs the fee.
        let rivalLanded: Outcome | undefined;
        const unsubscribeRival = await rival.tx.send(({ dispatchError, status }) => {
            if (status.isInBlock) {
                rivalLanded = dispatchError ? 'failed' : 'success';
            }
        });
        expect(await send(lower.tx, attestor.keyring, nonce)).toEqual('success');

        // Give the pool a few blocks to drop the rival.
        const rivalHash = rival.tx.hash.toHex();
        for (let i = 0; i < 20 && rivalLanded === undefined; i++) {
            const inPool = await api.rpc.author.pendingExtrinsics();
            if (!inPool.some((ex) => ex.hash.toHex() === rivalHash)) {
                break;
            }
            await new Promise((resolve) => setTimeout(resolve, 3_000));
        }
        unsubscribeRival();

        // Without the extension check this is 'failed': included, rejected in dispatch with
        // `InvalidAttestationContinuityProofTail`, full fee charged.
        expect(rivalLanded).toBeUndefined();
        const pending = await api.rpc.author.pendingExtrinsics();
        expect(pending.map((ex) => ex.hash.toHex())).not.toContain(rivalHash);

        // The runtime itself now calls the rival stale, so no node will include it.
        const validity = await api.call.taggedTransactionQueue.validateTransaction(
            'External',
            rival.tx.toU8a(),
            (await api.rpc.chain.getHeader()).hash,
        );
        expect(validity.isErr && validity.asErr.isInvalid && validity.asErr.asInvalid.isStale).toBe(true);

        // Successful commits are fee-free and the rival never reached dispatch.
        expect(await freeBalance()).toEqual(balanceBefore);
        expect(await lastDigest()).toEqual(u8aToHex(lower.digest));

        // Control: the same height built on the new head is admitted and commits.
        const rebuilt = commitOn(2 * INTERVAL, INTERVAL, lower.digest);
        expect(await send(rebuilt.tx, attestor.keyring)).toEqual('success');
        expect(await lastDigest()).toEqual(u8aToHex(rebuilt.digest));
    }, 120_000);
});
