import { newApi, ApiPromise } from '../../lib';
import { forElapsedBlocks } from '../utils';
import {
    chain_Anvil3_Id,
    chain_Anvil3_Name,
    chain_Anvil3_Key,
} from '../blockchain-tests/pallets/supported-chains/consts';

describe('Register Anvil 3', () => {
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // note: sudo will silently error when executed again
        await api.tx.sudo
            .sudo(
                api.tx.supportedChains.registerChain(
                    chain_Anvil3_Id,
                    chain_Anvil3_Name,
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                ),
            )
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 60_000);

    afterAll(async () => {
        await api.disconnect();
    });

    it('Anvil 3 is registered', async () => {
        const anvil3_Key = (await api.query.supportedChains.chainIdAndNameToUniqKey(chain_Anvil3_Id, chain_Anvil3_Name))
            .unwrap()
            .toNumber();

        // chain_Anvil3_Key is used in other tests so it must match reality
        // however when this script is executed for a second time (not as part of CI job setup)
        // and this 2nd execution happens *AFTER* handleEventCheckpointsCleared() the new value
        // will be different b/c handleEventCheckpointsCleared() will remove Anvil 3 and beforeAll()
        // will register it again!
        expect(anvil3_Key).toBeGreaterThanOrEqual(chain_Anvil3_Key);
    });
});
