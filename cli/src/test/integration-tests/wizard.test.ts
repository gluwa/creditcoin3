import { commandSync } from 'execa';
import { parseAmount } from '../../commands/options';
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
    it('new validator should appear as waiting after running', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Fund stash and controller
        const stash = randomTestAccount();

        const fundTx = await fundAddressesFromSudo([stash.address], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Run wizard setup with 1k ctc ang to pair with node Bob
        commandSync(`node ${CLI_PATH} wizard --amount 1000 --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus?.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus?.waiting);

        await api.disconnect();
    }, 120000);

    it('new validator should appear as waiting after running wizard with a proxy', async () => {
        const { api } = await newApi(ALICE_NODE_URL);

        // Fund stash and controller
        const stash = randomTestAccount();
        const proxy = randomTestAccount();

        const fundTx = await fundAddressesFromSudo([stash.address, proxy.address], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Bond with the stash first (Staking proxies cannot bond from scratch, but can bond-extra)
        commandSync(`node ${CLI_PATH} bond --amount 1000`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });

        // Add a staking proxy
        commandSync(`node ${CLI_PATH} proxy add --proxy ${proxy.address} --type Staking`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });
        // Run wizard setup with 1k ctc ang to pair with node Bob
        commandSync(`node ${CLI_PATH} wizard --url ${BOB_NODE_URL} --use-proxy ${stash.address}`, {
            env: {
                CC_PROXY_SECRET: proxy.secret,
            },
        });

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus?.bonded).toBe(true);
        expect(validatorStatus?.validating).toBe(true);
        expect(validatorStatus?.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus?.waiting);

        await api.disconnect();
    }, 120000);
});
