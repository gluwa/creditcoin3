import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';
import { signSendAndWatchCcKeyring, TxStatus } from '../../../../lib/tx';
import { CallerKeyring } from '../../../../lib/account/keyring';

describe('RegisterAttestor', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;
    let sudo: KeyringPair;
    /** Restored after the fee test — `register_attestor` checks attest-coin balance vs min bond. */
    let previousMinBond: string | undefined;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        sudo = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        previousMinBond = (await api.query.attestation.minBondRequirement(chain_Anvil2_Key)).toString();
        const zeroMin = api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, '0');
        const sudoTx = api.tx.sudo.sudo(zeroMin);
        const sudoKeyring: CallerKeyring = { type: 'caller', pair: sudo };
        const r = await signSendAndWatchCcKeyring(sudoTx, api, sudoKeyring);
        expect(r.status).toEqual(TxStatus.ok);
    });

    afterAll(async () => {
        try {
            if (api && sudo && previousMinBond !== undefined) {
                const restore = api.tx.attestation.setMinBondRequirement(chain_Anvil2_Key, previousMinBond);
                const sudoTx = api.tx.sudo.sudo(restore);
                const sudoKeyring: CallerKeyring = { type: 'caller', pair: sudo };
                await signSendAndWatchCcKeyring(sudoTx, api, sudoKeyring);
            }
        } finally {
            await api?.disconnect();
        }
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(alice.address);
        return new Promise((resolve, reject): void => {
            const attrAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');

            // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
            const unsubscribe = api.tx.attestation
                .registerAttestor(chain_Anvil2_Key, attrAccount.address)
                .signAndSend(alice, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
