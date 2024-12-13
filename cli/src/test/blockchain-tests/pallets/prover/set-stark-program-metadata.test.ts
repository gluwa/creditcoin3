import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';
import { starkProgramHash, starkProgramVersion } from './consts';

describe('SetStarkProgramMetadata', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // remove metadata b/c its set in genesis
        // will fail silently if already removed
        await api.tx.sudo.sudo(api.tx.prover.removeStarkProgramMetadata(starkProgramVersion)).signAndSend(root);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.prover.setStarkProgramMetadata(starkProgramVersion, starkProgramHash))
                .signAndSend(root, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
