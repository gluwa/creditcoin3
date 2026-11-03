import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import type { EventRecord } from '../../../../lib';
import { parseAmount } from '../../../../commands/options';
import { proofOfPossessionMessage } from '../../../../lib/attestor/proof-of-possession';
import { randomTestAccount, TestAccount } from '../../../integration-tests/helpers';
import { describeIf, dispatchErrorName, sudoInBlock } from '../../../utils';

// Unregistering an attestor unbonds its stash's bond into the stash's unlocking queue. When that
// queue is full, the matured chunks are released first and a same-era unbond merges into the
// last chunk, so a stash that never calls `withdraw_unbonded` can't get stuck with `NoMoreChunks`.
//
// Filling the queue for real takes several eras, so each case writes the stash's ledger with
// `system.setStorage` instead, then unregisters as the stash via `utility.dispatchAs`, or kicks
// the stash's active attestors as the operator, which is how the queue was hit on devnet.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'Unregister attestor with a full unlocking queue', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let maxChunks: number;
    let bondingDuration: number;
    // A throwaway chain, so attestors registered here don't touch the Anvil chains.
    let chainKey: number;

    const CHUNK = parseAmount('1');

    const currentEra = async (blockHash?: string): Promise<number> => {
        const query = blockHash ? (await api.at(blockHash)).query : api.query;
        const era = await query.staking.currentEra();
        return era.isSome ? era.unwrap().toNumber() : 0;
    };

    const asAccount = (account: TestAccount, call: Parameters<typeof api.tx.utility.dispatchAs>[1]) =>
        api.tx.utility.dispatchAs({ system: { signed: account.address } }, call);

    // The first failure among the `utility.dispatchAs` results in `events`, as a `section.Name`
    // error, or `null` when they all succeeded.
    const dispatchedAsError = (events: EventRecord[]): string | null => {
        let dispatched = false;
        for (const { event } of events) {
            if (api.events.utility.DispatchedAs.is(event)) {
                dispatched = true;
                const { result } = event.data;
                if (result.isErr) return dispatchErrorName(api, result.asErr);
            }
        }
        if (!dispatched) throw new Error('no DispatchedAs event');
        return null;
    };

    // `attest` for `attestor` with a fresh BLS key and a proof of possession bound to it.
    const attest = (attestor: TestAccount) => {
        const blsKey = WasmPrivateKey.generate(new TextEncoder().encode(attestor.secret));
        const publicKey = blsKey.public_key().as_bytes();
        const proof = blsKey.sign(proofOfPossessionMessage(chainKey, attestor.address, publicKey)).as_bytes();
        return asAccount(attestor, api.tx.attestation.attest(chainKey, publicKey, proof));
    };

    const stashEvents = (events: EventRecord[], stash: TestAccount) =>
        events
            .map(({ event }) => event)
            .filter(
                (event) =>
                    (api.events.attestation.Withdrawn.is(event) || api.events.attestation.Unbonded.is(event)) &&
                    event.data.stash.eq(stash.address),
            )
            .map((event) => ({ method: event.method, amount: event.data[1].toString() }));

    const ledgerOf = async (stash: TestAccount) => (await api.query.attestation.ledger(stash.address)).unwrap();

    // Registers fresh attestors for a fresh stash, gives the stash an unlocking queue with one
    // chunk of `CHUNK` per era from `chunkEras(currentEra)`, then unregisters the attestors. The
    // last two run in one extrinsic so they see the same era; if the era turns before it's
    // included, the chunk eras no longer mean what the case intends, so the case starts over.
    //
    // By default the stash unregisters one idle attestor. With `kick`, the attestors are made
    // active first (attest + forced election) and the operator kicks each with `unregister`.
    const unregisterWithQueue = async (
        chunkEras: (era: number) => number[],
        { attestorCount = 1, kick = false }: { attestorCount?: number; kick?: boolean } = {},
    ) => {
        for (let attempt = 0; attempt < 3; attempt++) {
            const stash = randomTestAccount();
            const attestors = Array.from({ length: attestorCount }, () => randomTestAccount());

            const setup = await sudoInBlock(
                api,
                root,
                api.tx.utility.batchAll([
                    api.tx.balances.forceSetBalance(stash.address, parseAmount('1000').toString()),
                    ...attestors.map((attestor) =>
                        asAccount(stash, api.tx.attestation.registerAttestor(chainKey, attestor.address)),
                    ),
                    ...(kick ? [...attestors.map(attest), api.tx.attestation.forceElection(chainKey)] : []),
                ]),
            );
            expect(dispatchErrorName(api, setup.innerError)).toBeNull();
            expect(dispatchedAsError(setup.events)).toBeNull();
            if (kick) {
                for (const attestor of attestors) {
                    const registered = await api.query.attestation.attestors(chainKey, attestor.address);
                    expect(registered.unwrap().status.isActive).toBe(true);
                }
            }

            const bond = (await ledgerOf(stash)).active.toBn();
            const era = await currentEra();
            const unlocking = chunkEras(era).map((chunkEra) => ({ value: CHUNK, era: chunkEra }));
            const seeded = api.createType('PalletAttestationLedgerAttestorLedger', {
                stash: stash.address,
                totalStaked: bond.add(CHUNK.muln(unlocking.length)),
                active: bond,
                unlocking,
            });

            const run = await sudoInBlock(
                api,
                root,
                api.tx.utility.batchAll([
                    api.tx.system.setStorage([[api.query.attestation.ledger.key(stash.address), seeded.toHex()]]),
                    ...attestors.map((attestor) =>
                        kick
                            ? api.tx.attestation.kickActiveAttestor(chainKey, attestor.address, true)
                            : asAccount(stash, api.tx.attestation.unregisterAttestor(chainKey, attestor.address)),
                    ),
                ]),
            );
            // A failed kick fails the whole batch; a failed `dispatchAs` only shows in its event.
            const error = kick ? dispatchErrorName(api, run.innerError) : dispatchedAsError(run.events);
            if (!kick) expect(dispatchErrorName(api, run.innerError)).toBeNull();

            if ((await currentEra(run.blockHash)) !== era) {
                console.log(`era turned after ${era} before the unregister was included, retrying`);
                continue;
            }
            return { stash, attestors, era, bond, seeded, error, events: run.events };
        }
        throw new Error('the era kept turning before the unregister was included');
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        maxChunks = api.consts.attestation.maxUnlockingChunks.toNumber();
        bondingDuration = api.consts.attestation.bondingDuration.toNumber();

        const chainId = Date.now();
        const registration = await sudoInBlock(
            api,
            root,
            api.tx.supportedChains.registerChain(
                chainId,
                `Unlocking queue ${chainId}`,
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
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('releases matured chunks to make room', async () => {
        // A full queue of chunks that all matured by the current era.
        const { stash, era, bond, error, events } = await unregisterWithQueue((current) =>
            Array.from({ length: maxChunks }, (_, i) => Math.max(0, current - (maxChunks - 1) + i)),
        );

        expect(error).toBeNull();
        expect(stashEvents(events, stash)).toEqual([
            { method: 'Withdrawn', amount: CHUNK.muln(maxChunks).toString() },
            { method: 'Unbonded', amount: bond.toString() },
        ]);

        const ledger = await ledgerOf(stash);
        expect(ledger.active.toBn().isZero()).toBe(true);
        expect(ledger.totalStaked.toBn().eq(bond)).toBe(true);
        expect(ledger.unlocking.map((chunk) => [chunk.value.toString(), chunk.era.toNumber()])).toEqual([
            [bond.toString(), era + bondingDuration],
        ]);
    }, 180_000);

    test('merges a same-era unbond into the last chunk', async () => {
        // A full queue of chunks that haven't matured, the last one in the era this unbond lands in.
        const { stash, era, bond, error, events } = await unregisterWithQueue((current) => [
            ...Array.from({ length: maxChunks - 1 }, () => current + 1),
            current + bondingDuration,
        ]);

        expect(error).toBeNull();
        expect(stashEvents(events, stash)).toEqual([{ method: 'Unbonded', amount: bond.toString() }]);

        const ledger = await ledgerOf(stash);
        expect(ledger.unlocking.length).toEqual(maxChunks);
        const last = ledger.unlocking[ledger.unlocking.length - 1];
        expect(last.era.toNumber()).toEqual(era + bondingDuration);
        expect(last.value.toBn().eq(CHUNK.add(bond))).toBe(true);
    }, 180_000);

    test('still fails with NoMoreChunks when nothing has matured and no chunk can be merged', async () => {
        // Synthetic: every chunk unlocks after this unbond's era, so none can be released or merged.
        const { stash, attestors, seeded, error, events } = await unregisterWithQueue((current) =>
            Array.from({ length: maxChunks }, () => current + bondingDuration + 1),
        );

        expect(error).toEqual('attestation.NoMoreChunks');
        expect(stashEvents(events, stash)).toEqual([]);
        expect((await ledgerOf(stash)).toHex()).toEqual(seeded.toHex());
        expect((await api.query.attestation.attestors(chainKey, attestors[0].address)).isSome).toBe(true);
    }, 180_000);

    test("the operator can kick a stash's active attestors in a row", async () => {
        // The devnet case: a full queue of matured chunks nobody withdrew, then the operator kicks
        // the stash's active attestors one after another.
        const { stash, attestors, era, bond, error, events } = await unregisterWithQueue(
            (current) => Array.from({ length: maxChunks }, (_, i) => Math.max(0, current - (maxChunks - 1) + i)),
            { attestorCount: 2, kick: true },
        );
        const perAttestor = bond.divn(attestors.length);

        expect(error).toBeNull();
        // The first kick releases the matured chunks; the second unbond merges into its chunk.
        expect(stashEvents(events, stash)).toEqual([
            { method: 'Withdrawn', amount: CHUNK.muln(maxChunks).toString() },
            { method: 'Unbonded', amount: perAttestor.toString() },
            { method: 'Unbonded', amount: perAttestor.toString() },
        ]);

        const chilled = events
            .map(({ event }) => event)
            .filter((event) => api.events.attestation.AttestorChilled.is(event) && event.data[0].eqn(chainKey))
            .map((event) => event.data[1].toString());
        expect(chilled).toEqual(attestors.map((attestor) => attestor.address));

        const active = (await api.query.attestation.activeAttestors(chainKey)).map((id) => id.toString());
        for (const attestor of attestors) {
            expect((await api.query.attestation.attestors(chainKey, attestor.address)).isNone).toBe(true);
            expect(active).not.toContain(attestor.address);
        }

        const ledger = await ledgerOf(stash);
        expect(ledger.active.toBn().isZero()).toBe(true);
        expect(ledger.totalStaked.toBn().eq(bond)).toBe(true);
        expect(ledger.unlocking.map((chunk) => [chunk.value.toString(), chunk.era.toNumber()])).toEqual([
            [bond.toString(), era + bondingDuration],
        ]);
    }, 180_000);
});
