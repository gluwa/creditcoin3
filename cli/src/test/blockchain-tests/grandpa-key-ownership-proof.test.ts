import { u8aConcat, u8aToHex } from '@polkadot/util';

import { newApi, ApiPromise } from '../../lib';

// Reporting a GRANDPA equivocation needs a proof that the offender's key belongs to a validator
// in the session it signed in. The runtime API that builds that proof was a stub returning `None`,
// so no equivocation could be reported, and `SetIdSession` kept too few entries to check one.
//
// The API proves whatever key it is given: for a key outside the validator set it returns a proof
// of absence, which `check_proof` rejects when a report is validated. So only the proof for a real
// authority is checked here.
describe('GRANDPA key ownership proofs', (): void => {
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test("a current authority's key gets a membership proof for the current session", async () => {
        const authorities = await api.call.grandpaApi.grandpaAuthorities();
        expect(authorities.length).toBeGreaterThan(0);
        const [authority] = authorities[0];
        const setId = await api.call.grandpaApi.currentSetId();

        // Called through `state_call` and decoded here: `api.call` hands back the raw result as the
        // `Option`'s value without decoding it, because the raw result is already `Bytes`.
        // The arguments go in as hex: a `Uint8Array` passed as `Bytes` is read as already
        // length-prefixed, which would send the wrong bytes.
        const raw = await api.rpc.state.call(
            'GrandpaApi_generate_key_ownership_proof',
            u8aToHex(u8aConcat(setId.toU8a(), authority.toU8a())),
        );
        const proof = api.createType('Option<Bytes>', raw.toU8a(true));

        expect(proof.isSome).toBe(true);
        // The opaque proof is an encoded session-historical `MembershipProof`.
        const membership = api.createType('SpSessionMembershipProof', proof.unwrap().toU8a(true));
        expect(membership.session.toNumber()).toEqual((await api.query.session.currentIndex()).toNumber());
        expect(membership.validatorCount.toNumber()).toEqual((await api.query.session.validators()).length);
        expect(membership.trieNodes.length).toBeGreaterThan(0);
    });

    test('set ids are kept for every session an offence can still be slashed in', () => {
        const bondingSessions =
            api.consts.staking.bondingDuration.toNumber() * api.consts.staking.sessionsPerEra.toNumber();
        expect(api.consts.grandpa.maxSetIdSessionEntries.toNumber()).toEqual(bondingSessions);
    });
});
