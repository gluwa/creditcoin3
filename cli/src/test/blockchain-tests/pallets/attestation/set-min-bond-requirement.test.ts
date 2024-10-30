import { BN } from '@polkadot/util';
import { newApi, ApiPromise, KeyringPair, MICROUNITS_PER_CTC } from '../../../../lib';
import { extractFee } from '../../../utils';

describe('SetMinBondRequirement', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const minBond = MICROUNITS_PER_CTC.mul(new BN(400));

        return new Promise((resolve, reject): void => {
            // WARNING: setMinBondRequirement() is global, not per supported chain !
            // This may lead to unwanted side effects in other test scenarios
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.attestation.setMinBondRequirement(minBond))
                .signAndSend(root, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
