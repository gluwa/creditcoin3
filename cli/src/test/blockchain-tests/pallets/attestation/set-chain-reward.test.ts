import { BN } from '@polkadot/util';
import { newApi, ApiPromise, KeyringPair, MICROUNITS_PER_CTC } from '../../../../lib';
import { extractFee } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('SetChainReward', (): void => {
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
        const chainReward = MICROUNITS_PER_CTC.mul(new BN(4444));

        return new Promise((resolve, reject): void => {
            // note: using chain Anvil2 b/c this may lead to side effects in other test scenarios
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.attestation.setChainReward(chain_Anvil2_Key, chainReward))
                .signAndSend(root, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
