import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { CallerKeyring } from '../../../../lib/account/keyring';
import { proofOfPossessionMessage } from '../../../../lib/attestor/proof-of-possession';
import { signSendAndWatchCcKeyring, TxResult, TxStatus } from '../../../../lib/tx';
import { randomFundedAccounts, TestAccount } from '../../../integration-tests/helpers';
import { describeIf } from '../../../utils';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../supported-chains/consts';

// A BLS proof of possession must be bound to the claiming account and chain, so a proof seen in
// someone else's public `attest` transaction can't be replayed to take their BLS key.
describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'Attest proof of possession', (): void => {
    let api: ApiPromise;
    // NOTE: Alice is the STASH for both random attestors on the Anvil2 chain
    let alice: KeyringPair;
    let victim: TestAccount;
    let thief: TestAccount;
    let victimBlsKey: WasmPrivateKey;
    let victimBlsPublicKey: Uint8Array;

    const blsKeyFor = (account: TestAccount) => WasmPrivateKey.generate(new TextEncoder().encode(account.secret));

    const boundProof = (key: WasmPrivateKey, chainKey: number, address: string): Uint8Array =>
        key.sign(proofOfPossessionMessage(chainKey, address, key.public_key().as_bytes())).as_bytes();

    const attest = (account: TestAccount, blsPublicKey: Uint8Array, proof: Uint8Array): Promise<TxResult> => {
        const signer: CallerKeyring = { type: 'caller', pair: account.keyring };
        return signSendAndWatchCcKeyring(api.tx.attestation.attest(chain_Anvil2_Key, blsPublicKey, proof), api, signer);
    };

    const expectInvalidProofOfPossession = (result: TxResult) => {
        expect(result.status).toEqual(TxStatus.failed);
        expect(result.info).toContain('attestation.InvalidProofOfPossession');
    };

    const blsKeyOwner = async (blsPublicKey: Uint8Array) =>
        await api.query.attestation.blsKeyOwner(chain_Anvil2_Key, blsPublicKey);

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        [victim, thief] = await randomFundedAccounts(api, root, 2);

        const register = api.tx.utility.batchAll([
            api.tx.attestation.registerAttestor(chain_Anvil2_Key, victim.address),
            api.tx.attestation.registerAttestor(chain_Anvil2_Key, thief.address),
        ]);
        const result = await signSendAndWatchCcKeyring(register, api, { type: 'caller', pair: alice });
        expect(result.status, `registering attestors failed: ${result.info}`).toEqual(TxStatus.ok);

        victimBlsKey = blsKeyFor(victim);
        victimBlsPublicKey = victimBlsKey.public_key().as_bytes();
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test("another controller can't front-run the victim with the victim's proof", async () => {
        const victimProof = boundProof(victimBlsKey, chain_Anvil2_Key, victim.address);

        expectInvalidProofOfPossession(await attest(thief, victimBlsPublicKey, victimProof));
        expect((await blsKeyOwner(victimBlsPublicKey)).isNone).toBe(true);
    }, 60_000);

    test('a proof over the bare public key (the old format) is rejected', async () => {
        const unboundProof = victimBlsKey.sign(victimBlsPublicKey).as_bytes();

        expectInvalidProofOfPossession(await attest(victim, victimBlsPublicKey, unboundProof));
        expect((await blsKeyOwner(victimBlsPublicKey)).isNone).toBe(true);
    }, 60_000);

    test('a proof bound to another chain is rejected', async () => {
        const otherChainProof = boundProof(victimBlsKey, chain_Anvil1_Key, victim.address);

        expectInvalidProofOfPossession(await attest(victim, victimBlsPublicKey, otherChainProof));
        expect((await blsKeyOwner(victimBlsPublicKey)).isNone).toBe(true);
    }, 60_000);

    test('a proof bound to the attestor and chain is accepted and claims the key', async () => {
        const victimProof = boundProof(victimBlsKey, chain_Anvil2_Key, victim.address);

        const result = await attest(victim, victimBlsPublicKey, victimProof);
        expect(result.status, `attest failed: ${result.info}`).toEqual(TxStatus.ok);

        const owner = await blsKeyOwner(victimBlsPublicKey);
        expect(owner.isSome).toBe(true);
        expect(owner.unwrap().eq(victim.address)).toBe(true);

        const attestor = await api.query.attestation.attestors(chain_Anvil2_Key, victim.address);
        expect(attestor.unwrap().status.isIdle).toBe(false);
    }, 60_000);

    test("the victim's proof still can't be replayed once the key is claimed", async () => {
        const victimProof = boundProof(victimBlsKey, chain_Anvil2_Key, victim.address);

        // The proof check runs before the key-uniqueness check, so this fails as a bad proof,
        // not as BlsKeyAlreadyRegistered.
        expectInvalidProofOfPossession(await attest(thief, victimBlsPublicKey, victimProof));
        expect((await blsKeyOwner(victimBlsPublicKey)).unwrap().eq(victim.address)).toBe(true);
    }, 60_000);
});
