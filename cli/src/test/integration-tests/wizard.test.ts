import { commandSync } from 'execa';
import { parseAmountInternal } from '../../lib/parsing';
import { signSendAndWatch } from '../../lib/tx';
import {
    randomTestAccount,
    fundAddressesFromSudo,
    ALICE_NODE_URL,
    BOB_NODE_URL,
    initAliceKeyring,
    CLI_PATH,
} from './helpers';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { newApi } from '../../lib';

describe('integration test: validator wizard setup', () => {
    it('new validator should appear as waiting after running %s', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Fund stash and controller
        const stash = randomTestAccount();

        const fundTx = await fundAddressesFromSudo([stash.address], parseAmountInternal('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Run wizard setup with 1k ctc ang to pair with node Bob
        commandSync(`node ${CLI_PATH} wizard --amount 1000 --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus.waiting);

        await api.disconnect();
    }, 120000);
});
