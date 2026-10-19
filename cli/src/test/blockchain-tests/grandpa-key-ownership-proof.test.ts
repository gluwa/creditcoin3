import { u8aToHex } from '@polkadot/util';
import { randomAsU8a } from '@polkadot/util-crypto';

import { newApi, ApiPromise } from '../../lib';

// Reporting a GRANDPA equivocation needs a proof that the offender's key belongs to a validator
// in the session it signed in. The runtime API that builds that proof was a stub returning `None`,
// so no equivocation could be reported, and `SetIdSession` kept too few entries to check one.
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

        const proof = await api.call.grandpaApi.generateKeyOwnershipProof(setId, authority);

        expect(proof.isSome).toBe(true);
        // The opaque proof is an encoded session-historical `MembershipProof`.
        const membership = api.createType('SpSessionMembershipProof', proof.unwrap().toU8a(true));
        expect(membership.session.toNumber()).toEqual((await api.query.session.currentIndex()).toNumber());
        expect(membership.validatorCount.toNumber()).toEqual((await api.query.session.validators()).length);
        expect(membership.trieNodes.length).toBeGreaterThan(0);
    });

    test('a key that is not an authority gets no proof', async () => {
        const setId = await api.call.grandpaApi.currentSetId();
        const proof = await api.call.grandpaApi.generateKeyOwnershipProof(setId, u8aToHex(randomAsU8a(32)));
        expect(proof.isNone).toBe(true);
    });

    test('set ids are kept for every session an offence can still be slashed in', () => {
        const bondingSessions =
            api.consts.staking.bondingDuration.toNumber() * api.consts.staking.sessionsPerEra.toNumber();
        expect(api.consts.grandpa.maxSetIdSessionEntries.toNumber()).toEqual(bondingSessions);
    });
});
