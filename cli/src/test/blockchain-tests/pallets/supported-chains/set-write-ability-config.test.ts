import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { extractFee, forElapsedBlocks } from '../../../utils';
import { describeIf } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'SetWriteAbilityConfig', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let chainKey: U64;

    const chainId = Date.now();
    const chainName = `Test Chain ${chainId}`;
    const encoding = 'V1';

    // 32-byte write-ability chain key (bytes32)
    const writeAbilityChainKey = '0x' + '11'.repeat(32);
    const messageAttestationEnabled = true;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        const nonce = await api.rpc.system.accountNextIndex(root.address);

        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(
                    chainId,
                    chainName,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    encoding,
                    null,
                ),
            )
            .signAndSend(root, { nonce });

        await forElapsedBlocks(api);

        chainKey = (await api.query.supportedChains.chainIdAndNameToUniqKey(chainId, chainName)).unwrap();
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('fee is min 0.01 CTC', async (): Promise<void> => {
        const nonce = await api.rpc.system.accountNextIndex(root.address);

        const fee = await new Promise((resolve, reject): void => {
            const unsubscribe = api.tx.sudo
                .sudo(
                    api.tx.supportedChains.setWriteAbilityConfig(
                        chainKey,
                        writeAbilityChainKey,
                        messageAttestationEnabled,
                    ),
                )
                .signAndSend(root, { nonce }, async ({ dispatchError, events, status }) => {
                    await extractFee(resolve, reject, unsubscribe, api, dispatchError, events, status);
                })
                .catch((error) => reject(new Error(error)));
        });

        expect(fee).toBeGreaterThanOrEqual((global as any).CREDITCOIN_MINIMUM_TXN_FEE);

        const stored = await api.query.supportedChains.writeAbilityConfigs(chainKey);

        expect(stored.isSome).toEqual(true);
        expect(stored.unwrap().writeAbilityChainKey.toString()).toEqual(writeAbilityChainKey);
        expect(stored.unwrap().messageAttestationEnabled.isTrue).toEqual(true);
    }, 30_000);
});
