import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import type { EventRecord } from '../../../../lib';
import { describeIf, dispatchErrorName, sudoInBlock } from '../../../utils';

// Registering a chain must enforce the same roster limits as `set_max_attestors` /
// `set_max_invulnerables`: each must be non-zero and at most `MaxAttestationNodes`.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'RegisterChain roster limits', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let maxAttestationNodes: number;

    // Unique chain ids, so a registration that wrongly succeeds can't collide with a later case.
    let nextChainId = Date.now();

    const registerChain = (chainId: number, maxAttestors: number | null, maxInvulnerables: number | null) =>
        sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Roster limits ${chainId}`,
                null,
                null,
                null,
                maxAttestors,
                maxInvulnerables,
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
        maxAttestationNodes = api.consts.attestation.maxAttestationNodes.toNumber();
    });

    afterAll(async () => {
        await api.disconnect();
    });

    const outOfRange: [string, (max: number) => [number | null, number | null]][] = [
        ['max_attestors = 0', () => [0, null]],
        ['max_attestors above MaxAttestationNodes', (max) => [max + 1, null]],
        ['max_invulnerables = 0', () => [null, 0]],
        ['max_invulnerables above MaxAttestationNodes', (max) => [null, max + 1]],
    ];

    test.each(outOfRange)(
        'rejects %s and leaves no chain behind',
        async (_, limits) => {
            const [maxAttestors, maxInvulnerables] = limits(maxAttestationNodes);
            const chainId = nextChainId++;
            const chainKeyBefore = await api.query.supportedChains.chainKeyValue();

            const { innerError, events } = await registerChain(chainId, maxAttestors, maxInvulnerables);

            // The attestation pallet rejects with a string error, which arrives as `Other`;
            // its message isn't encoded on-chain.
            expect(innerError?.isOther).toBe(true);
            expect(registeredChainKey(events)).toBeUndefined();
            // The chain insert and key bump are rolled back with the failed call.
            expect((await api.query.supportedChains.chainKeyValue()).eq(chainKeyBefore)).toBe(true);
            expect(
                (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, `Roster limits ${chainId}`)).isNone,
            ).toBe(true);
        },
        60_000,
    );

    test('accepts both limits at MaxAttestationNodes and stores them', async () => {
        const chainId = nextChainId++;

        const { innerError, events } = await registerChain(chainId, maxAttestationNodes, maxAttestationNodes);

        expect(dispatchErrorName(api, innerError)).toBeNull();
        const chainKey = registeredChainKey(events);
        expect(chainKey).toBeDefined();
        expect((await api.query.attestation.maxAttestors(chainKey!)).toNumber()).toEqual(maxAttestationNodes);
        expect((await api.query.attestation.maxInvulnerables(chainKey!)).toNumber()).toEqual(maxAttestationNodes);
    }, 60_000);
});
