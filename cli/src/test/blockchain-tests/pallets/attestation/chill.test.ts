import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair, BN, MICROUNITS_PER_CTC } from '../../../../lib';
import { fundFromSudo } from '../../../integration-tests/helpers';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';
import { describeIf } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'Chill', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;
    let sudo: KeyringPair;
    let attestorAccount: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        sudo = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
        attestorAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        await fundFromSudo(api, attestorAccount.address, MICROUNITS_PER_CTC.mul(new BN(2000)));
        let nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestorAccount.address)
            .signAndSend(alice, { nonce });

        // wait for at least one block b/c when registerAttestor() & attest() happen to be in the same
        // block attest() can fail b/c storage hasn't been updated yet!
        await forElapsedBlocks(api, { minBlocks: 1 });

        const blsSecretKey = WasmPrivateKey.generate(attestorAccount.secret);
        const blsPublicKey = blsSecretKey.public_key().as_bytes();
        const proofOfPossession = blsSecretKey.sign(blsPublicKey);
        nonce = await api.rpc.system.accountNextIndex(attestorAccount.address);
        await api.tx.attestation
            .attest(chain_Anvil2_Key, blsPublicKey, proofOfPossession.as_bytes())
            .signAndSend(attestorAccount.keyring, { nonce });
        await forElapsedBlocks(api, { minBlocks: 1 });

        nonce = await api.rpc.system.accountNextIndex(sudo.address);
        await api.tx.sudo.sudo(api.tx.attestation.forceElection(1)).signAndSend(sudo, { nonce });
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(alice.address);
        return new Promise((resolve, reject): void => {
            // NOTE: this is signed by the stash (Alice)
            const unsubscribe = api.tx.attestation
                .chill(chain_Anvil2_Key, attestorAccount.address)
                .signAndSend(alice, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 60_000);
});
