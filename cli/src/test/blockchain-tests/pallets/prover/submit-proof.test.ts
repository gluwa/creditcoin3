import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee } from '../../../utils';

describe('SubmitProof', (): void => {
    let api: ApiPromise;
    let signer: KeyringPair;
    let root: KeyringPair;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        signer = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const stark_program_hash = '617734937651202173';
        const version = 1;
        await api.tx.sudo.sudo(api.tx.prover.setStarkProgramMetadata(stark_program_hash, version)).signAndSend(root);
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        // note: these are dummy values used only to extract the fee
        const proof = '0x012345';
        const query = {
            chainId: 0,
            height: 0,
            index: 0,
            layoutSegments: [
                {
                    offset: 0,
                    size: 0,
                },
            ],
        };

        return new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.prover
                .submitProof(proof, query)
                .signAndSend(signer, { nonce: -1 }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(error));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);
});
