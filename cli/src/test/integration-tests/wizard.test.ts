import { commandSync } from 'execa';
import { newApi } from '../../api';
import { parseAmountInternal } from '../../lib/parsing';
import { signSendAndWatch } from '../../lib/tx';
import {
    randomTestAccount,
    fundAddressesFromSudo,
    ALICE_NODE_URL,
    BOB_NODE_URL,
    initAlithKeyring,
} from './helpers';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';

describe('integration test: validator wizard setup', () => {
    it('new validator should appear as waiting after running %s', async () => {
        // Fund stash and controller
        const stash = randomTestAccount();
        const controller = randomTestAccount();

        const fundTx = await fundAddressesFromSudo(
            [stash.address, controller.address],
            parseAmountInternal('10000')
        );
        const { api } = await newApi(ALICE_NODE_URL);
        await signSendAndWatch(fundTx, api, initAlithKeyring());

        // Run wizard setup with 1k ctc ang to pair with node Bob
        commandSync(
            `node dist/index.js wizard --amount 1000 --url ${BOB_NODE_URL}`,
            {
                env: {
                    CC_STASH_SECRET: stash.secret,
                    CC_CONTROLLER_SECRET: controller.secret,
                },
            }
        );

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus.waiting);

        await api.disconnect();
    }, 120000);
});
