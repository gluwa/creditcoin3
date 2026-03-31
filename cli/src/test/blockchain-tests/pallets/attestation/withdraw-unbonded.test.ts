import { newApi, ApiPromise, KeyringPair, BN, MICROUNITS_PER_CTC } from '../../../../lib';
import { fundFromSudo, waitEras } from '../../../integration-tests/helpers';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';
import { describeIf } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'WithdrawUnbonded', (): void => {
    let api: ApiPromise;
    let alice: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        // NOTE: Alice acts as the STASH for a random attestor on the Anvil2 chain
        const attestorAccount = (global as any).CREDITCOIN_CREATE_SIGNER('random');
        await fundFromSudo(api, attestorAccount.address, MICROUNITS_PER_CTC.mul(new BN(2000)));
        let nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestorAccount.address)
            .signAndSend(alice, { nonce });

        // wait for Attestors storage item to be updated!
        await forElapsedBlocks(api, { minBlocks: 1 });

        // unregister so that unbonding can begin
        nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.attestation
            .unregisterAttestor(chain_Anvil2_Key, attestorAccount.address)
            .signAndSend(alice, { nonce });

        // wait for funds to be unlocked!
        const unbondingPeriod: number = api.consts.attestation.bondingDuration.toNumber();
        await waitEras(unbondingPeriod, api); // 5 minutes
    }, 400_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(alice.address);
        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.attestation
                .withdrawUnbonded()
                .signAndSend(alice, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
