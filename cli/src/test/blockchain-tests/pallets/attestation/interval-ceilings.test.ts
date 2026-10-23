import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import type { EventRecord } from '../../../../lib';
import { describeIf, dispatchErrorName, sudoInBlock } from '../../../utils';

// The checkpoint interval and the chain attestation interval are capped by runtime ceilings
// (`MaxAttestationCheckpointInterval`, `MaxChainAttestationInterval`), both in their setters and
// when a chain is registered with them. `MaxCatchup` has its own ceiling (`MaxCatchupCeiling`).
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'Attestation interval ceilings', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let maxCheckpointInterval: number;
    let maxAttestationInterval: number;
    let maxCatchupCeiling: number;
    // The largest attestation interval the setters and registerChain accept: the ceiling, or
    // DefaultMaxCatchup when that is lower.
    let acceptedAttestationInterval: number;
    // A throwaway chain, so the Anvil chains other tests attest on keep their configuration.
    let chainKey: number;

    // Unique chain ids, so a registration that wrongly succeeds can't collide with a later case.
    let nextChainId = Date.now();

    const registerChain = (chainId: number, attestationInterval: number | null, checkpointInterval: number | null) =>
        sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Interval ceilings ${chainId}`,
                null,
                attestationInterval,
                checkpointInterval,
                null,
                null,
                null,
                'V1',
                null,
            ),
        );

    const registeredChainKey = (events: EventRecord[]): number | undefined => {
        for (const { event } of events) {
            if (api.events.supportedChains.ChainRegistered.is(event)) {
                return event.data.chainKey.toNumber();
            }
        }
        return undefined;
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        maxCheckpointInterval = api.consts.attestation.maxAttestationCheckpointInterval.toNumber();
        maxAttestationInterval = api.consts.attestation.maxChainAttestationInterval.toNumber();
        maxCatchupCeiling = api.consts.attestation.maxCatchupCeiling.toNumber();
        acceptedAttestationInterval = Math.min(
            maxAttestationInterval,
            api.consts.attestation.defaultMaxCatchup.toNumber(),
        );

        const { innerError, events } = await registerChain(nextChainId++, null, null);
        expect(dispatchErrorName(api, innerError)).toBeNull();
        chainKey = registeredChainKey(events)!;
        expect(chainKey).toBeDefined();
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('setAttestationsPerCheckpoint', () => {
        test('rejects a value above MaxAttestationCheckpointInterval', async () => {
            const before = await api.query.attestation.attestationCheckpointInterval(chainKey);

            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setAttestationsPerCheckpoint(chainKey, maxCheckpointInterval + 1),
            );

            expect(dispatchErrorName(api, innerError)).toEqual('attestation.InvalidAttestationsPerCheckpoint');
            expect((await api.query.attestation.attestationCheckpointInterval(chainKey)).eq(before)).toBe(true);
        }, 60_000);

        test('accepts MaxAttestationCheckpointInterval', async () => {
            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setAttestationsPerCheckpoint(chainKey, maxCheckpointInterval),
            );

            expect(dispatchErrorName(api, innerError)).toBeNull();
            expect((await api.query.attestation.attestationCheckpointInterval(chainKey)).toNumber()).toEqual(
                maxCheckpointInterval,
            );
        }, 60_000);
    });

    describe('setChainAttestationInterval', () => {
        test('rejects a value above MaxChainAttestationInterval', async () => {
            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setChainAttestationInterval(chainKey, maxAttestationInterval + 1),
            );

            expect(dispatchErrorName(api, innerError)).toEqual('attestation.InvalidAttestationInterval');
            expect((await api.query.attestation.pendingAttestationInterval(chainKey)).isNone).toBe(true);
        }, 60_000);

        test('accepts the largest allowed attestation interval', async () => {
            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setChainAttestationInterval(chainKey, acceptedAttestationInterval),
            );

            expect(dispatchErrorName(api, innerError)).toBeNull();
            // The setter stages the new interval; it's applied on a later update.
            const pending = await api.query.attestation.pendingAttestationInterval(chainKey);
            expect(pending.unwrap().toNumber()).toEqual(acceptedAttestationInterval);
        }, 60_000);
    });

    describe('setMaxCatchup', () => {
        test('rejects a value above MaxCatchupCeiling', async () => {
            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setMaxCatchup(chainKey, maxCatchupCeiling + 1),
            );

            expect(dispatchErrorName(api, innerError)).toEqual('attestation.InvalidMaxCatchup');
            expect((await api.query.attestation.pendingMaxCatchup(chainKey)).isNone).toBe(true);
        }, 60_000);

        test('accepts MaxCatchupCeiling', async () => {
            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setMaxCatchup(chainKey, maxCatchupCeiling),
            );

            expect(dispatchErrorName(api, innerError)).toBeNull();
            // The setter stages the new bound; it's applied on a later update.
            const pending = await api.query.attestation.pendingMaxCatchup(chainKey);
            expect(pending.unwrap().toNumber()).toEqual(maxCatchupCeiling);
        }, 60_000);
    });

    describe('registerChain', () => {
        const outOfRange: [
            string,
            (maxAttestation: number, maxCheckpoint: number) => [number | null, number | null],
        ][] = [
            [
                'an attestation interval above MaxChainAttestationInterval',
                (maxAttestation) => [maxAttestation + 1, null],
            ],
            [
                'a checkpoint interval above MaxAttestationCheckpointInterval',
                (_, maxCheckpoint) => [null, maxCheckpoint + 1],
            ],
        ];

        test.each(outOfRange)(
            'rejects %s and leaves no chain behind',
            async (_, intervals) => {
                const [attestationInterval, checkpointInterval] = intervals(
                    maxAttestationInterval,
                    maxCheckpointInterval,
                );
                const chainId = nextChainId++;
                const chainKeyBefore = await api.query.supportedChains.chainKeyValue();

                const { innerError, events } = await registerChain(chainId, attestationInterval, checkpointInterval);

                // The attestation pallet rejects with a string error, which arrives as `Other`;
                // its message isn't encoded on-chain.
                expect(innerError?.isOther).toBe(true);
                expect(registeredChainKey(events)).toBeUndefined();
                expect((await api.query.supportedChains.chainKeyValue()).eq(chainKeyBefore)).toBe(true);
            },
            60_000,
        );

        test('accepts both intervals at their largest allowed values and stores them', async () => {
            const { innerError, events } = await registerChain(
                nextChainId++,
                acceptedAttestationInterval,
                maxCheckpointInterval,
            );

            expect(dispatchErrorName(api, innerError)).toBeNull();
            const registered = registeredChainKey(events);
            expect(registered).toBeDefined();
            expect((await api.query.attestation.chainAttestationInterval(registered!)).toNumber()).toEqual(
                acceptedAttestationInterval,
            );
            expect((await api.query.attestation.attestationCheckpointInterval(registered!)).toNumber()).toEqual(
                maxCheckpointInterval,
            );
        }, 60_000);
    });
});
