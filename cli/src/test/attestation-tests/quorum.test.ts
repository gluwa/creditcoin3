import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { dispatchErrorName, sudoInBlock } from '../utils';

// An attestation's quorum is 2/3 + 1 of the chain's active attestors. It used to be measured
// against min(active attestors, TargetSampleSize), so lowering the target sample size let a
// minority of the active set attest on its own.
describe('Attestation quorum', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let originalTargetSampleSize: number;

    // Chain 2's interval stays fixed during the suite, so attestations keep arriving at a steady rate.
    const chainKey = chain_Anvil1_Key;
    const ATTESTATIONS_TO_CHECK = 3;

    const quorum = (activeAttestors: number) => Math.floor((activeAttestors * 2) / 3) + 1;

    const setTargetSampleSize = async (size: number) => {
        const staged = await sudoInBlock(api, root, api.tx.attestation.setTargetSampleSize(chainKey, size));
        expect(dispatchErrorName(api, staged.innerError)).toBeNull();
        // Applied at the next epoch boundary unless forced.
        const applied = await sudoInBlock(api, root, api.tx.attestation.forceApplyUpdates());
        expect(dispatchErrorName(api, applied.innerError)).toBeNull();
        return applied.blockHash;
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        originalTargetSampleSize = (await api.query.attestation.targetSampleSize(chainKey)).toNumber();
    }, 60_000);

    afterAll(async () => {
        await setTargetSampleSize(originalTargetSampleSize);
        await api.disconnect();
    }, 60_000);

    test(
        'a target sample size below the active set does not lower the quorum',
        async () => {
            const active = (await api.query.attestation.activeAttestors(chainKey)).length;
            expect(active).toBeGreaterThanOrEqual(3);

            const appliedAt = await setTargetSampleSize(1);
            expect((await (await api.at(appliedAt)).query.attestation.targetSampleSize(chainKey)).toNumber()).toEqual(
                1,
            );
            const appliedAtNumber = (await api.rpc.chain.getHeader(appliedAt)).number.toNumber();

            // Each attestation of chain 2 committed after the change, with its signer count and
            // the active-set size it was committed against.
            const signed: { headerNumber: number; signers: number; active: number }[] = [];
            await new Promise<void>((resolve, reject) => {
                let unsubscribe: (() => void) | undefined;
                api.rpc.chain
                    .subscribeNewHeads(async (header) => {
                        if (signed.length >= ATTESTATIONS_TO_CHECK || header.number.toNumber() <= appliedAtNumber) {
                            return;
                        }
                        try {
                            const at = await api.at(header.hash);
                            for (const { event } of await at.query.system.events()) {
                                if (!api.events.attestation.BlockAttested.is(event)) continue;
                                const [eventChainKey, headerNumber, digest] = event.data;
                                if (!eventChainKey.eq(chainKey)) continue;

                                const attestation = (
                                    await at.query.attestation.attestations(chainKey, digest)
                                ).unwrap();
                                signed.push({
                                    headerNumber: headerNumber.toNumber(),
                                    signers: attestation.attestors.length,
                                    active: (await at.query.attestation.activeAttestors(chainKey)).length,
                                });
                            }
                            if (signed.length >= ATTESTATIONS_TO_CHECK) {
                                unsubscribe?.();
                                resolve();
                            }
                        } catch (error) {
                            unsubscribe?.();
                            reject(error instanceof Error ? error : new Error(String(error)));
                        }
                    })
                    .then((unsub) => {
                        unsubscribe = unsub;
                    })
                    .catch(reject);
            });

            console.log(`attestations after TargetSampleSize = 1: ${JSON.stringify(signed)}`);
            for (const { signers, active: activeThen } of signed) {
                // With the old rule a single signer was enough.
                expect(signers).toBeGreaterThanOrEqual(quorum(activeThen));
            }
        },
        15 * 60_000,
    );
});
