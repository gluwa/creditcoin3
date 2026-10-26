import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { dispatchErrorName, extractFee, sudoInBlock } from '../../../utils';
import { chain_Anvil2_Key } from '../supported-chains/consts';

describe('SetMaxInvulnerables', (): void => {
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
        const nonce = await api.rpc.system.accountNextIndex(root.address);
        return new Promise((resolve, reject): void => {
            // note: using chain Anvil2 b/c this may lead to side effects in other test scenarios
            const unsubscribe = api.tx.sudo
                .sudo(api.tx.attestation.setMaxInvulnerables(chain_Anvil2_Key, 4))
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        }).then((fee) => {
            expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);
        });
    }, 30_000);

    // Same bounds as `set_max_attestors` and chain registration: non-zero, at most `MaxAttestationNodes`.
    test.each([
        ['0', () => 0],
        ['above MaxAttestationNodes', (ceiling: number) => ceiling + 1],
    ])(
        'rejects %s',
        async (_, newMax) => {
            const before = await api.query.attestation.maxInvulnerables(chain_Anvil2_Key);

            const { innerError } = await sudoInBlock(
                api,
                root,
                api.tx.attestation.setMaxInvulnerables(
                    chain_Anvil2_Key,
                    newMax(api.consts.attestation.maxAttestationNodes.toNumber()),
                ),
            );

            expect(dispatchErrorName(api, innerError)).toEqual('attestation.InvalidMaxInvulnerables');
            expect((await api.query.attestation.maxInvulnerables(chain_Anvil2_Key)).eq(before)).toBe(true);
        },
        60_000,
    );
});
