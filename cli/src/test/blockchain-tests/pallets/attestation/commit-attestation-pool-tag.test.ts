import { SubmittableExtrinsic } from '@polkadot/api/types';
import { u8aToHex } from '@polkadot/util';
import { randomAsU8a } from '@polkadot/util-crypto';
import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { proofOfPossessionMessage } from '../../../../lib/attestor/proof-of-possession';
import { parseAmount } from '../../../../commands/options';
import { randomTestAccount, TestAccount } from '../../../integration-tests/helpers';
import { describeIf, dispatchErrorName, sudoInBlock } from '../../../utils';

// The pool's `provides` tag for a `commit_attestation` is an exclusive slot per
// `(chain_key, digest)`. Only a submission that passes full `validate_attestation` may hold it;
// otherwise an active attestor could park a forged commit there and crowd out the honest one.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'commitAttestation pool tag', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let alice: KeyringPair;
    // The only active attestor on a throwaway chain, so the test holds every key it needs.
    let attestor: TestAccount;
    let blsKey: WasmPrivateKey;
    let chainKey: number;

    // Every tag the extension grants starts with its SCALE-encoded identifier.
    let tagPrefix: string;

    // Signs and sends `tx`, resolving once it's in a block; rejects with `section.Name` if it fails.
    const signedInBlock = (tx: SubmittableExtrinsic<'promise'>, signer: KeyringPair): Promise<void> =>
        new Promise((resolve, reject) => {
            let unsubscribe: (() => void) | undefined;
            tx.signAndSend(signer, { nonce: -1 }, ({ dispatchError, status }) => {
                if (dispatchError) {
                    unsubscribe?.();
                    if (dispatchError.isModule) {
                        const { section, name } = api.registry.findMetaError(dispatchError.asModule);
                        reject(new Error(`${section}.${name}`));
                    } else {
                        reject(new Error(dispatchError.toString()));
                    }
                } else if (status.isInBlock) {
                    unsubscribe?.();
                    resolve();
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
        tagPrefix = u8aToHex(api.createType('Text', 'PrevalidateAttestationCommit').toU8a());

        const chainId = Date.now();
        const registration = await sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Pool tag ${chainId}`,
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

        attestor = randomTestAccount();
        const funding = await sudoInBlock(
            api,
            root,
            api.tx.balances.forceSetBalance(attestor.address, parseAmount('1000').toString()),
        );
        expect(dispatchErrorName(api, funding.innerError)).toBeNull();

        // NOTE: Alice is the STASH for the attestor
        await signedInBlock(api.tx.attestation.registerAttestor(chainKey, attestor.address), alice);

        blsKey = WasmPrivateKey.generate(new TextEncoder().encode(attestor.secret));
        const blsPublicKey = blsKey.public_key().as_bytes();
        const proofOfPossession = blsKey
            .sign(proofOfPossessionMessage(chainKey, attestor.address, blsPublicKey))
            .as_bytes();
        await signedInBlock(api.tx.attestation.attest(chainKey, blsPublicKey, proofOfPossession), attestor.keyring);

        const election = await sudoInBlock(api, root, api.tx.attestation.forceElection(chainKey));
        expect(dispatchErrorName(api, election.innerError)).toBeNull();

        const active = await api.query.attestation.activeAttestors(chainKey);
        expect(active.length).toEqual(1);
        expect(active[0].eq(attestor.address)).toBe(true);
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('a commit from an active attestor that fails full validation gets no tag', async () => {
        // Passes the pool's cheap checks: the signer is active, the attestor list is within bounds,
        // the continuity proof is empty and nothing is attested on this chain yet. But the
        // aggregate signature is over the wrong message, so `validate_attestation` rejects it.
        const forgedCommit = api.tx.attestation.commitAttestation({
            attestation: {
                chainKey,
                headerNumber: 1,
                headerHash: randomAsU8a(32),
                root: randomAsU8a(32),
                prevDigest: null,
            },
            signature: blsKey.sign(randomAsU8a(32)).as_bytes(),
            attestors: [attestor.address],
            continuityProof: { lowerEndpointDigest: new Uint8Array(32), roots: [] },
        });
        await forgedCommit.signAsync(attestor.keyring, { nonce: -1 });

        const bestHash = (await api.rpc.chain.getHeader()).hash;
        const validity = await api.call.taggedTransactionQueue.validateTransaction(
            'External',
            forgedCommit.toHex(),
            bestHash,
        );

        // Still admitted, so it pays its fee in dispatch, which is what deters spam...
        expect(validity.isOk).toBe(true);
        // ...but without the exclusive slot. Other extensions' tags (e.g. the nonce) remain.
        const tags = validity.asOk.provides.map((tag) => u8aToHex(tag));
        expect(tags.length).toBeGreaterThan(0);
        expect(tags.filter((tag) => tag.startsWith(tagPrefix))).toEqual([]);
    }, 60_000);
});
